//! Offline measurement of per-cell upscale reconstruction rules -- artifacts/design/
//! UPSCALE-RECONSTRUCTION-2026-09-14.md.
//!
//! The shipped shader draws at `n = S*m` while simulating at `S`, reconstructing each render
//! pixel with mask-aware bilinear interpolation (`sandart-render/src/shader.wgsl`, ~line 622-670).
//! Bilinear spills ~1/8 of a cell's content (1D) into each empty neighbour, which is drawn as
//! extra width -- this is measured to widen falling water streams far more than the underlying
//! simulated stream actually is.
//!
//! This is a MEASUREMENT tool, not a shipped change: it never touches the shader, the renderer,
//! the wasm crate, or physics. It takes a real 512 snapshot, downscales it by block-average to
//! 256 (m=2) and 128 (m=4) using the *coarse resolution's own rasterized mask* (never a resampling
//! of the fine mask), then re-upscales with eight candidate rules and compares each against the
//! original 512 field. Comparing separate 256/512 simulations would conflate reconstruction error
//! with the fact that different resolutions evolve at different rates -- see CLAUDE.md's method
//! note. Every rule takes ONLY (height field, mask) as input -- no material/wetness branch --
//! satisfying "the same rule for all materials" by construction, not by convention.
//!
//! **Round 4 (this revision) adds R7** and the interior/frontier false-negative split that
//! motivated it -- see the R7 section below and
//! `artifacts/design/UPSCALE-RECONSTRUCTION-2026-09-14.md` round-4 note.
//!
//! Run: `cargo run -p sandart-sim --release --example diag_upscale_reconstruction`
//!
//! Writes PNG crops to `artifacts/design/upscale-2026-09-14/` (resolved relative to this crate's
//! `CARGO_MANIFEST_DIR`, so it works regardless of the caller's cwd) and prints all metric tables
//! to stdout.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, MASK_INSIDE, MASK_OUTSIDE};

const THRESH: f32 = 0.003;

/// Round 5's chosen sharpening factor for R7's `emptiness = clamp(k*(1-h_min/h0), 0, 1)`, used by
/// every ALL_RULES/PICTURE_RULES table and picture from here on (i.e. "R7" in this file's output
/// means "R7 at this k" unless a table explicitly sweeps k). See `run_r7_k_sweep` and
/// UPSCALE-RECONSTRUCTION-2026-09-14.md §10 for how this value was chosen.
const R7_K: f32 = 1.0; // placeholder pending the sweep in this run; updated below once measured.

// ---------------------------------------------------------------------------------------------
// Scene construction -- ports of examples/profile_sandfall_water.rs's `build()`.
// ---------------------------------------------------------------------------------------------

fn step(sim: &mut DrawingSimulation) {
    let (r, m, s) = (sim.marble_radius, sim.material_mode, sim.sandbox_shape);
    // 0.0 frame times keep the adaptive budget controller from perturbing budget_n mid-run.
    sim.update(1.0 / 60.0, &[None; 5], r, m, s, 0.0, 0.0);
}

fn fill_upper_half(sim: &mut DrawingSimulation, level: f32) {
    let w = sim.heightmap.width;
    for y in 0..w / 2 {
        for x in 0..w {
            let i = y * w + x;
            if sim.shape_mask[i] != 0 {
                sim.heightmap.data[i] = level;
            }
        }
    }
}

/// Scenario (a): MultiNeckHourglass, Water, mid-drain with streams active.
fn build_water_multineck() -> DrawingSimulation {
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = SandboxShape::MultiNeckHourglass;
    sim.apply_preset(MaterialMode::Water);
    sim.generate_shape_mask();
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = 2.5;
    sim.budget_n = 128;
    sim.active_bounds.active = true;
    fill_upper_half(&mut sim, 0.5);
    sim
}

/// Scenario (b): Hourglass, DrySand, draining -- a visible pile and slopes.
fn build_dry_hourglass() -> DrawingSimulation {
    let mut sim = DrawingSimulation::new();
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.apply_preset(MaterialMode::DrySand);
    sim.generate_shape_mask();
    sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
    sim.lateral_substeps = 1.0;
    sim.budget_n = 128;
    sim.active_bounds.active = true;
    fill_upper_half(&mut sim, 0.5);
    sim
}

struct Snapshot {
    name: &'static str,
    h: Vec<f32>,
    mask512: Vec<u8>,
    sim: DrawingSimulation,
}

fn snapshot_a() -> Snapshot {
    let mut sim = build_water_multineck();
    for _ in 0..600 {
        step(&mut sim);
    }
    Snapshot { name: "a_water_multineck_mid_drain", h: sim.heightmap.data.clone(), mask512: sim.shape_mask.clone(), sim }
}

fn snapshot_b() -> Snapshot {
    let mut sim = build_dry_hourglass();
    for _ in 0..900 {
        step(&mut sim);
    }
    Snapshot { name: "b_dry_hourglass_pile", h: sim.heightmap.data.clone(), mask512: sim.shape_mask.clone(), sim }
}

/// Scenario (c): same run as (a), but the reported field is an EMA with alpha 0.4 over the last
/// ~15 ticks -- `y = alpha*current + (1-alpha)*y` per cell, matching sandart-wasm's
/// `update_and_upload_ema` / `temporal_alpha` default (see sandart-wasm/src/lib.rs). This is what
/// the deployed page actually displays, not the raw per-tick field.
fn snapshot_c_ema() -> Snapshot {
    const ALPHA: f32 = 0.4;
    const WINDOW: u32 = 15;
    let mut sim = build_water_multineck();
    for _ in 0..(600 - WINDOW) {
        step(&mut sim);
    }
    let mut ema = sim.heightmap.data.clone();
    for _ in 0..WINDOW {
        step(&mut sim);
        for i in 0..ema.len() {
            ema[i] = ALPHA * sim.heightmap.data[i] + (1.0 - ALPHA) * ema[i];
        }
    }
    Snapshot { name: "c_water_multineck_ema_alpha0.4", h: ema, mask512: sim.shape_mask.clone(), sim }
}

// ---------------------------------------------------------------------------------------------
// Downscale: block-average over the block's fine cells that are inside the FINE mask, reported
// against the COARSE resolution's own rasterized mask (never a resample of the fine mask).
// ---------------------------------------------------------------------------------------------

struct CoarseField {
    size: usize,
    h: Vec<f32>,
    mask: Vec<u8>,
}

impl CoarseField {
    #[inline]
    fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.size && (y as usize) < self.size
            && self.mask[y as usize * self.size + x as usize] != MASK_OUTSIDE
    }
    #[inline]
    fn get(&self, x: i32, y: i32) -> f32 {
        self.h[y as usize * self.size + x as usize]
    }
}

struct DownscaleReport {
    mass_lost: f64,
    total_mass: f64,
}

fn downscale(fine_h: &[f32], fine_mask: &[u8], fine_size: usize, coarse_mask: &[u8], coarse_size: usize, m: usize) -> (CoarseField, DownscaleReport) {
    let mut h = vec![0.0f32; coarse_size * coarse_size];
    let mut mass_lost = 0.0f64;
    let mut total_mass = 0.0f64;
    for cy in 0..coarse_size {
        for cx in 0..coarse_size {
            let mut sum = 0.0f64;
            let mut cnt = 0usize;
            let coarse_outside = coarse_mask[cy * coarse_size + cx] == MASK_OUTSIDE;
            for dy in 0..m {
                for dx in 0..m {
                    let fx = cx * m + dx;
                    let fy = cy * m + dy;
                    let fi = fy * fine_size + fx;
                    if fine_mask[fi] == MASK_OUTSIDE {
                        continue;
                    }
                    let val = fine_h[fi] as f64;
                    total_mass += val;
                    if coarse_outside {
                        mass_lost += val;
                    } else {
                        sum += val;
                        cnt += 1;
                    }
                }
            }
            h[cy * coarse_size + cx] = if cnt > 0 { (sum / cnt as f64) as f32 } else { 0.0 };
        }
    }
    (CoarseField { size: coarse_size, h, mask: coarse_mask.to_vec() }, DownscaleReport { mass_lost, total_mass })
}

// ---------------------------------------------------------------------------------------------
// Coordinate convention shared by every rule: fine pixel i's continuous position in coarse-cell
// units, matching the shader's `texel_coords = uv*sim_size - 0.5` exactly (see shader.wgsl line
// 624: with uv=(i+0.5)/render_size and render_size = sim_size*m, texel_coords = (i+0.5)/m - 0.5).
// ---------------------------------------------------------------------------------------------

#[inline]
fn fine_to_coarse(i: usize, m: usize) -> f32 {
    (i as f32 + 0.5) / m as f32 - 0.5
}

#[inline]
fn clamp_idx(i: i32, size: usize) -> usize {
    i.clamp(0, size as i32 - 1) as usize
}

// ---------------------------------------------------------------------------------------------
// R0: nearest (piecewise constant).
// ---------------------------------------------------------------------------------------------

fn r0_sample(coarse: &CoarseField, xc: f32, yc: f32) -> f32 {
    let cx = clamp_idx(xc.round() as i32, coarse.size);
    let cy = clamp_idx(yc.round() as i32, coarse.size);
    if coarse.mask[cy * coarse.size + cx] != MASK_OUTSIDE { coarse.h[cy * coarse.size + cx] } else { 0.0 }
}

// ---------------------------------------------------------------------------------------------
// R1: mask-aware bilinear, exactly as the shader computes h_center (shader.wgsl lines 628-670):
// standard 2x2-texel bilinear, weights of OUTSIDE corners zeroed and the remainder renormalised.
// This is what already ships; NOT per-cell conservative (mass mixes across the cell boundary),
// though it is globally conservative and overshoot-free.
// ---------------------------------------------------------------------------------------------

fn r1_sample(coarse: &CoarseField, xc: f32, yc: f32) -> f32 {
    let ix0 = xc.floor() as i32;
    let iy0 = yc.floor() as i32;
    let fx = xc - ix0 as f32;
    let fy = yc - iy0 as f32;
    let x0 = clamp_idx(ix0, coarse.size);
    let x1 = clamp_idx(ix0 + 1, coarse.size);
    let y0 = clamp_idx(iy0, coarse.size);
    let y1 = clamp_idx(iy0 + 1, coarse.size);
    let at = |x: usize, y: usize| -> (f32, bool) { (coarse.h[y * coarse.size + x], coarse.mask[y * coarse.size + x] != MASK_OUTSIDE) };
    let (h00, in00) = at(x0, y0);
    let (h10, in10) = at(x1, y0);
    let (h01, in01) = at(x0, y1);
    let (h11, in11) = at(x1, y1);
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;
    let iw00 = if in00 { w00 } else { 0.0 };
    let iw10 = if in10 { w10 } else { 0.0 };
    let iw01 = if in01 { w01 } else { 0.0 };
    let iw11 = if in11 { w11 } else { 0.0 };
    let wsum = (iw00 + iw10 + iw01 + iw11).max(1e-4);
    (h00 * iw00 + h10 * iw10 + h01 * iw01 + h11 * iw11) / wsum
}

// ---------------------------------------------------------------------------------------------
// Shared 3x3-stencil machinery for R2/R3/R4: least-squares gradient, Barth-Jespersen limiter,
// bisection re-solve for exact per-cell conservation after clipping at 0.
// ---------------------------------------------------------------------------------------------

/// Least-squares plane gradient over the available (inside) 3x3 neighbours, excluding OUTSIDE
/// cells from the fit entirely -- a wall never contributes a fake zero. Falls back to independent
/// per-axis normal equations when the neighbour set is rank-deficient (e.g. only two neighbours
/// along one line, common right next to a wall).
fn ls_gradient(coarse: &CoarseField, cx: i32, cy: i32) -> (f32, f32) {
    let h0 = coarse.get(cx, cy);
    let (mut sxx, mut syy, mut sxy, mut sxr, mut syr) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for ddy in -1..=1i32 {
        for ddx in -1..=1i32 {
            if ddx == 0 && ddy == 0 {
                continue;
            }
            let (nx, ny) = (cx + ddx, cy + ddy);
            if coarse.inside(nx, ny) {
                let dv = coarse.get(nx, ny) - h0;
                let (fx, fy) = (ddx as f32, ddy as f32);
                sxx += fx * fx;
                syy += fy * fy;
                sxy += fx * fy;
                sxr += fx * dv;
                syr += fy * dv;
            }
        }
    }
    let det = sxx * syy - sxy * sxy;
    if det.abs() < 1e-6 {
        let gx = if sxx > 1e-6 { sxr / sxx } else { 0.0 };
        let gy = if syy > 1e-6 { syr / syy } else { 0.0 };
        (gx, gy)
    } else {
        ((sxr * syy - syr * sxy) / det, (syr * sxx - sxr * sxy) / det)
    }
}

/// R3's raw (pre-limit) gradient: per axis, steepen toward whichever available neighbour is
/// FULLER (higher) than this cell so the plane reaches that neighbour's exact value at the
/// shared face (offset 0.5, half the 1-cell spacing to the neighbour's own centre) -- the user's
/// "match the gradient" idea. Where this cell is a local max along an axis (no fuller neighbour),
/// falls back to a plain (non-steepened) one-sided/central difference, giving a gentle downhill
/// slope off a peak rather than a fabricated steep one.
fn r3_axis_gradient(coarse: &CoarseField, cx: i32, cy: i32, axx: i32, axy: i32, h0: f32) -> f32 {
    let plus = if coarse.inside(cx + axx, cy + axy) { Some(coarse.get(cx + axx, cy + axy)) } else { None };
    let minus = if coarse.inside(cx - axx, cy - axy) { Some(coarse.get(cx - axx, cy - axy)) } else { None };
    match (plus, minus) {
        (Some(hp), Some(hm)) => {
            if hp >= h0 && hp >= hm {
                2.0 * (hp - h0)
            } else if hm >= h0 && hm > hp {
                -2.0 * (hm - h0)
            } else {
                0.5 * (hp - hm)
            }
        }
        (Some(hp), None) => if hp > h0 { 2.0 * (hp - h0) } else { hp - h0 },
        (None, Some(hm)) => if hm > h0 { -2.0 * (hm - h0) } else { -(hm - h0) },
        (None, None) => 0.0,
    }
}

fn r3_gradient(coarse: &CoarseField, cx: i32, cy: i32) -> (f32, f32) {
    let h0 = coarse.get(cx, cy);
    (r3_axis_gradient(coarse, cx, cy, 1, 0, h0), r3_axis_gradient(coarse, cx, cy, 0, 1, h0))
}

/// 3x3 min/max INCLUDING the cell's own value, over inside neighbours only.
fn neighbour_min_max(coarse: &CoarseField, cx: i32, cy: i32) -> (f32, f32) {
    let h0 = coarse.get(cx, cy);
    let (mut mn, mut mx) = (h0, h0);
    for ddy in -1..=1i32 {
        for ddx in -1..=1i32 {
            if ddx == 0 && ddy == 0 {
                continue;
            }
            if coarse.inside(cx + ddx, cy + ddy) {
                let v = coarse.get(cx + ddx, cy + ddy);
                mn = mn.min(v);
                mx = mx.max(v);
            }
        }
    }
    (mn, mx)
}

/// Barth-Jespersen limiter: the single scalar `phi` (applied to the whole gradient vector, not
/// per-axis -- this is what keeps the limiter from undoing R3's face-matching in the direction
/// away from the empty side while still enforcing it) that keeps the plane's value at all four of
/// the cell's own corners inside `[neighbour_min, neighbour_max]`.
fn bj_phi(h0: f32, gx: f32, gy: f32, mn: f32, mx: f32) -> f32 {
    let mut phi = 1.0f32;
    for &(dx, dy) in &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
        let extrap = h0 + gx * dx + gy * dy;
        let d = extrap - h0;
        if extrap > mx && d.abs() > 1e-9 {
            phi = phi.min(((mx - h0) / d).max(0.0));
        } else if extrap < mn && d.abs() > 1e-9 {
            phi = phi.min(((mn - h0) / d).max(0.0));
        }
    }
    phi.clamp(0.0, 1.0)
}

/// Bisection over a vertical shift `delta`, applied AFTER clip-at-0, so the mean over the cell's
/// own m x m sample grid equals `h0` exactly. `sample_mean` is monotone non-decreasing in `delta`
/// (clipping at 0 only ever flattens the low end), so bisection is exact.
fn resolve_conservation(h0: f32, gx: f32, gy: f32, phi: f32, m: usize) -> f32 {
    let sample_mean = |delta: f32| -> f32 {
        let mut sum = 0.0f32;
        for j in 0..m {
            for i in 0..m {
                let dx = (i as f32 + 0.5) / m as f32 - 0.5;
                let dy = (j as f32 + 0.5) / m as f32 - 0.5;
                sum += (h0 + phi * gx * dx + phi * gy * dy + delta).max(0.0);
            }
        }
        sum / (m * m) as f32
    };
    let (mut lo, mut hi) = (-2.0f32, 2.0f32);
    for _ in 0..48 {
        let mid = 0.5 * (lo + hi);
        if sample_mean(mid) < h0 { lo = mid; } else { hi = mid; }
    }
    0.5 * (lo + hi)
}

enum CellModel {
    Plane { h0: f32, gx: f32, gy: f32, phi: f32, delta: f32 },
    Plic { nx: f32, ny: f32, t: f32, h_ref: f32, full: bool },
}

fn precompute_plane_models(coarse: &CoarseField, m: usize, steepen: bool) -> Vec<Option<CellModel>> {
    let n = coarse.size;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                out.push(None);
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            let (gx_raw, gy_raw) = if steepen { r3_gradient(coarse, cx, cy) } else { ls_gradient(coarse, cx, cy) };
            let (mn, mx) = neighbour_min_max(coarse, cx, cy);
            let phi = bj_phi(h0, gx_raw, gy_raw, mn, mx);
            let delta = resolve_conservation(h0, gx_raw, gy_raw, phi, m);
            out.push(Some(CellModel::Plane { h0, gx: gx_raw, gy: gy_raw, phi, delta }));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// R4: vertical-edge PLIC (Youngs). Interface normal from the same 3x3 least-squares gradient;
// covered fraction f = h0 / h_ref, h_ref = max over inside 3x3 neighbours (including self). The
// covered region is the half of the unit cell on the fuller-neighbour side of a line with that
// normal, positioned (via exact half-plane polygon clipping + bisection, not a Monte-Carlo
// sample) so its area fraction is exactly f. Material is h_ref there, 0 elsewhere --
// conservative by construction: covered_area * h_ref = f * h_ref = h0, no re-solve needed.
// ---------------------------------------------------------------------------------------------

fn clip_halfplane(poly: &[(f32, f32)], nx: f32, ny: f32, t: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    let n = poly.len();
    for i in 0..n {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % n];
        let d1 = nx * x1 + ny * y1 - t;
        let d2 = nx * x2 + ny * y2 - t;
        let (in1, in2) = (d1 >= 0.0, d2 >= 0.0);
        if in1 {
            out.push((x1, y1));
        }
        if in1 != in2 {
            let tt = d1 / (d1 - d2);
            out.push((x1 + (x2 - x1) * tt, y1 + (y2 - y1) * tt));
        }
    }
    out
}

fn polygon_area(poly: &[(f32, f32)]) -> f32 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut a = 0.0f32;
    for i in 0..poly.len() {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        a += x1 * y2 - x2 * y1;
    }
    (a * 0.5).abs()
}

fn area_frac_exact(nx: f32, ny: f32, t: f32) -> f32 {
    let square = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
    polygon_area(&clip_halfplane(&square, nx, ny, t))
}

fn solve_plic_threshold(nx: f32, ny: f32, f: f32) -> f32 {
    let (mut lo, mut hi) = (-1.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        // area_frac_exact is non-increasing in t (a higher threshold admits less area).
        if area_frac_exact(nx, ny, mid) > f { lo = mid; } else { hi = mid; }
    }
    0.5 * (lo + hi)
}

fn precompute_plic_models(coarse: &CoarseField) -> Vec<Option<CellModel>> {
    let n = coarse.size;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                out.push(None);
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            // neighbour_min_max already includes the cell's own value in its max.
            let (_, h_ref) = neighbour_min_max(coarse, cx, cy);
            let h_ref = h_ref.max(1e-6);
            let f = (h0 / h_ref).clamp(0.0, 1.0);
            if f >= 1.0 - 1e-6 {
                out.push(Some(CellModel::Plic { nx: 0.0, ny: -1.0, t: 0.0, h_ref: h0, full: true }));
                continue;
            }
            let (gx, gy) = ls_gradient(coarse, cx, cy);
            let gmag = (gx * gx + gy * gy).sqrt();
            let (nx, ny) = if gmag > 1e-6 { (gx / gmag, gy / gmag) } else { (0.0, -1.0) };
            let t = solve_plic_threshold(nx, ny, f);
            out.push(Some(CellModel::Plic { nx, ny, t, h_ref, full: false }));
        }
    }
    out
}

fn eval_model(model: &Option<CellModel>, dx: f32, dy: f32) -> f32 {
    match model {
        None => 0.0,
        Some(CellModel::Plane { h0, gx, gy, phi, delta }) => (h0 + phi * gx * dx + phi * gy * dy + delta).max(0.0),
        Some(CellModel::Plic { nx, ny, t, h_ref, full }) => {
            if *full || nx * dx + ny * dy >= *t { *h_ref } else { 0.0 }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// R5: anti-aliased PLIC coverage (an exact clipped-area fraction of each render PIXEL's own
// footprint, not a point sample of the interface) times a limited-plane height. Round-2 addition
// -- see UPSCALE-RECONSTRUCTION-2026-09-14.md revision history. Fixes R4's failure mode (a thin
// covered strip can fall entirely between two point samples and vanish): here every pixel gets a
// continuous coverage in [0,1] instead of a binary in/out, so a strip narrower than one pixel
// fades rather than disappearing, and the pixel MEANS used for conservation are exact by
// construction rather than only exact in the point-sample limit.
// ---------------------------------------------------------------------------------------------

/// Area fraction of the axis-aligned box `[cx-half,cx+half] x [cy-half,cy+half]` on the covered
/// side (`n.p >= t`) of the interface line -- the same exact half-plane clip `area_frac_exact`
/// uses for the whole unit cell, generalised to one render pixel's own smaller footprint. This
/// closed-form clipped-area computation is exactly what an analytic-AA fragment shader would do
/// per pixel (see §7 of the writeup) -- no bisection, no sampling.
fn box_area_frac(nx: f32, ny: f32, t: f32, cx: f32, cy: f32, half: f32) -> f32 {
    let poly = [(cx - half, cy - half), (cx + half, cy - half), (cx + half, cy + half), (cx - half, cy + half)];
    let area = polygon_area(&clip_halfplane(&poly, nx, ny, t));
    let box_area = (2.0 * half) * (2.0 * half);
    if box_area > 1e-12 { (area / box_area).clamp(0.0, 1.0) } else { 0.0 }
}

struct R5Model { nx: f32, ny: f32, t: f32, h0: f32, gx: f32, gy: f32, phi: f32, delta: f32 }

/// Per-cell R5 parameters. `delta` is solved in CLOSED FORM (no bisection): the mean over the
/// cell's own `m*m` pixels of `coverage(dx,dy) * (h0 + phi*gx*dx + phi*gy*dy + delta)` must equal
/// `h0`. Since `coverage` doesn't depend on `delta`, this is linear in `delta`:
/// `delta = (h0 - mean(coverage*plane)) / mean(coverage)`. Unlike R2/R3, there is no clip-then-
/// resolve nonlinearity here -- the "edge" is carried entirely by `coverage`, not by clipping the
/// height to 0, so the height field itself never needs clamping mid-solve (final output is
/// clamped defensively at 0, but the conservation identity above is exact before that clamp).
fn precompute_r5_models(coarse: &CoarseField, m: usize) -> Vec<Option<R5Model>> {
    let n = coarse.size;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                out.push(None);
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            let (_, h_ref) = neighbour_min_max(coarse, cx, cy);
            let h_ref = h_ref.max(1e-6);
            let f = (h0 / h_ref).clamp(0.0, 1.0);
            let (gx, gy) = ls_gradient(coarse, cx, cy);
            let (mn, mx) = neighbour_min_max(coarse, cx, cy);
            let phi = bj_phi(h0, gx, gy, mn, mx);
            let gmag = (gx * gx + gy * gy).sqrt();
            let (nx, ny) = if gmag > 1e-6 { (gx / gmag, gy / gmag) } else { (0.0, -1.0) };
            let t = solve_plic_threshold(nx, ny, f);
            let half = 0.5 / m as f32;
            let (mut sum_cov, mut sum_cov_h) = (0.0f32, 0.0f32);
            for j in 0..m {
                for i in 0..m {
                    let dx = (i as f32 + 0.5) / m as f32 - 0.5;
                    let dy = (j as f32 + 0.5) / m as f32 - 0.5;
                    let cov = box_area_frac(nx, ny, t, dx, dy, half);
                    let plane_h = h0 + phi * gx * dx + phi * gy * dy;
                    sum_cov += cov;
                    sum_cov_h += cov * plane_h;
                }
            }
            let mean_cov = sum_cov / (m * m) as f32;
            let mean_cov_h = sum_cov_h / (m * m) as f32;
            let delta = if mean_cov > 1e-6 { (h0 - mean_cov_h) / mean_cov } else { 0.0 };
            out.push(Some(R5Model { nx, ny, t, h0, gx, gy, phi, delta }));
        }
    }
    out
}

/// Returns `(height_shown, raw_coverage)` -- the first is what feeds rms/mass/width metrics and
/// the picture (the coverage-weighted, i.e. "as displayed", height); the second is the RAW
/// coverage fraction in `[0,1]`, which is what the coverage/IoU metric thresholds at 0.5 (per the
/// round-2 brief) instead of thresholding a height.
fn eval_r5(model: &Option<R5Model>, dx: f32, dy: f32, m: usize) -> (f32, f32) {
    match model {
        None => (0.0, 0.0),
        Some(md) => {
            let half = 0.5 / m as f32;
            let cov = box_area_frac(md.nx, md.ny, md.t, dx, dy, half);
            let plane_h = (md.h0 + md.phi * md.gx * dx + md.phi * md.gy * dy + md.delta).max(0.0);
            (cov * plane_h, cov)
        }
    }
}

fn reconstruct_r5(coarse: &CoarseField, m: usize) -> (Vec<f32>, Vec<f32>) {
    let n = coarse.size * m;
    let models = precompute_r5_models(coarse, m);
    let mut h_out = vec![0.0f32; n * n];
    let mut cov_out = vec![0.0f32; n * n];
    for fy in 0..n {
        let yc = fine_to_coarse(fy, m);
        let cy = clamp_idx(yc.round() as i32, coarse.size);
        let dy = yc - cy as f32;
        for fx in 0..n {
            let xc = fine_to_coarse(fx, m);
            let cx = clamp_idx(xc.round() as i32, coarse.size);
            let dx = xc - cx as f32;
            let (hv, cv) = eval_r5(&models[cy * coarse.size + cx], dx, dy, m);
            h_out[fy * n + fx] = hv;
            cov_out[fy * n + fx] = cv;
        }
    }
    (h_out, cov_out)
}

// ---------------------------------------------------------------------------------------------
// R6 (round 3): R5 plus a centred-strip case for features one cell wide. R4/R5's single 2D
// interface line comes from a GRADIENT direction, which is ill-defined exactly when it should
// matter most: a stream with empty space on both sides has opposing left/right gradients that
// cancel, so the "normal" is numerically ~0 and the line lands wherever floating-point noise
// points it -- the isolated flecks in §4.3 of the writeup. R6 replaces the gradient-direction
// normal with a per-AXIS confinement signal that doesn't have this cancellation problem: an axis
// bounded by two near-empty (or wall) neighbours is "confined" regardless of whether they're
// exactly equal, and the strip is centred there instead of picking an arbitrary side.
// ---------------------------------------------------------------------------------------------

/// Fraction of the 1D interval `[pixel_center-pixel_half, pixel_center+pixel_half]` covered by
/// `[center-half_width, center+half_width]` -- the exact 1D analogue of `box_area_frac`'s 2D clip,
/// used because an R6 strip is, by construction, unbounded (full coverage) along whichever axis
/// isn't the confined one.
fn overlap_1d(center: f32, half_width: f32, pixel_center: f32, pixel_half: f32) -> f32 {
    let lo = (pixel_center - pixel_half).max(center - half_width);
    let hi = (pixel_center + pixel_half).min(center + half_width);
    if pixel_half <= 0.0 { return 0.0; }
    ((hi - lo).max(0.0) / (2.0 * pixel_half)).clamp(0.0, 1.0)
}

/// Confinement (0 = this axis just continues the flat flow, i.e. both neighbours read like the
/// cell's own value; 1 = this axis shows a real feature -- a flush-full/empty edge OR both
/// neighbours empty) and signed bias (-1 = negative-side neighbour fuller, +1 = positive-side
/// neighbour fuller) along one axis.
///
/// Confinement is deliberately measured as DEVIATION FROM `h0` (`|a-h0| + |b-h0|`, normalised by
/// `h_ref`), not as "how empty the neighbours are relative to `h_ref`" (`1 - avg/h_ref`, tried
/// first and rejected): the latter compares BOTH axes against the same global `h_ref`, which is
/// often set by a neighbour on the OTHER axis entirely, and so wrongly flags a flat flow-through
/// axis (neighbours equal to `h0`, i.e. genuinely open) as "confined" whenever `h_ref` happens to
/// be much larger than `h0` -- confirmed wrong by the `SELFTEST=1` R5-vs-R6 check, which needs
/// this deviation form to reduce to R5 in the single-axis-dominant case it's built to test.
///
/// Deliberately the ONE place in this whole instrument that reads an OUTSIDE (wall) neighbour as
/// height 0 -- everywhere else (gradients, `h_ref`, Barth-Jespersen bounds) a wall is excluded
/// entirely per the "walls are not empty" invariant, but for THIS shape signal a wall confines a
/// stream exactly the way empty space does (visually, a stream squeezed against a wall on one
/// side and open on the other is the same "confined axis" as a stream with empty cells on both
/// sides), and the neck of an hourglass -- squeezed between two walls -- is exactly the
/// degenerate case this rule exists to fix.
fn axis_confinement_bias(coarse: &CoarseField, cx: i32, cy: i32, axx: i32, axy: i32, h0: f32, h_ref: f32) -> (f32, f32) {
    let eff = |x: i32, y: i32| -> f32 { if coarse.inside(x, y) { coarse.get(x, y) } else { 0.0 } };
    let a = eff(cx - axx, cy - axy);
    let b = eff(cx + axx, cy + axy);
    let h_ref = h_ref.max(1e-6);
    let confinement = (((a - h0).abs() + (b - h0).abs()) / h_ref).clamp(0.0, 1.0);
    let bias = ((b - a) / h_ref).clamp(-1.0, 1.0);
    (confinement, bias)
}

struct R6Model { h0: f32, gx: f32, gy: f32, phi: f32, delta: f32, off_x: f32, off_y: f32, strip_half: f32, w: f32 }

/// `w` blends the x-strip and y-strip coverage fields as `w*cov_x + (1-w)*cov_y` -- a SMOOTH
/// combination (no if/else branch on which axis "wins"), so the output varies continuously as
/// axis confinement shifts from one axis to the other. Both `cov_x` and `cov_y` individually
/// average to `f` over the cell's footprint (each is a single strip of width `f`), so ANY blend
/// weight preserves that average -- conservation doesn't depend on `w` being "correct", only on
/// the closed-form `delta` solve below, exactly as R5.
///
/// In the pure single-axis-dominant case (one neighbour on an axis at `h_ref`, the opposite one at
/// 0, and the other axis open/unconfined) this reduces EXACTLY to R5: `off = bias*(1-f)/2` at
/// `bias=1` places the strip flush against that edge, `[0.5-f, 0.5]`, identical to R5's half-plane
/// with an axis-aligned normal at the threshold that gives area `f`. It does NOT reduce to R5 for
/// a genuinely diagonal interface (R6 has no rotated-line case at all) -- see the writeup for why
/// that trade is accepted.
fn precompute_r6_models(coarse: &CoarseField, m: usize) -> Vec<Option<R6Model>> {
    let n = coarse.size;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                out.push(None);
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            let (_, h_ref) = neighbour_min_max(coarse, cx, cy);
            let h_ref = h_ref.max(1e-6);
            let f = (h0 / h_ref).clamp(0.0, 1.0);
            let (gx, gy) = ls_gradient(coarse, cx, cy);
            let (mn, mx) = neighbour_min_max(coarse, cx, cy);
            let phi = bj_phi(h0, gx, gy, mn, mx);
            let (conf_x, bias_x) = axis_confinement_bias(coarse, cx, cy, 1, 0, h0, h_ref);
            let (conf_y, bias_y) = axis_confinement_bias(coarse, cx, cy, 0, 1, h0, h_ref);
            let wsum = conf_x + conf_y;
            let w = if wsum > 1e-6 { conf_x / wsum } else { 0.5 };
            let strip_half = f / 2.0;
            let off_x = bias_x * (1.0 - f) / 2.0;
            let off_y = bias_y * (1.0 - f) / 2.0;
            let half = 0.5 / m as f32;
            let (mut sum_cov, mut sum_cov_h) = (0.0f32, 0.0f32);
            for j in 0..m {
                for i in 0..m {
                    let dx = (i as f32 + 0.5) / m as f32 - 0.5;
                    let dy = (j as f32 + 0.5) / m as f32 - 0.5;
                    let cov_x = overlap_1d(off_x, strip_half, dx, half);
                    let cov_y = overlap_1d(off_y, strip_half, dy, half);
                    let cov = w * cov_x + (1.0 - w) * cov_y;
                    let plane_h = h0 + phi * gx * dx + phi * gy * dy;
                    sum_cov += cov;
                    sum_cov_h += cov * plane_h;
                }
            }
            let mean_cov = sum_cov / (m * m) as f32;
            let mean_cov_h = sum_cov_h / (m * m) as f32;
            let delta = if mean_cov > 1e-6 { (h0 - mean_cov_h) / mean_cov } else { 0.0 };
            out.push(Some(R6Model { h0, gx, gy, phi, delta, off_x, off_y, strip_half, w }));
        }
    }
    out
}

fn eval_r6(model: &Option<R6Model>, dx: f32, dy: f32, m: usize) -> (f32, f32) {
    match model {
        None => (0.0, 0.0),
        Some(md) => {
            let half = 0.5 / m as f32;
            let cov_x = overlap_1d(md.off_x, md.strip_half, dx, half);
            let cov_y = overlap_1d(md.off_y, md.strip_half, dy, half);
            let cov = md.w * cov_x + (1.0 - md.w) * cov_y;
            let plane_h = (md.h0 + md.phi * md.gx * dx + md.phi * md.gy * dy + md.delta).max(0.0);
            (cov * plane_h, cov)
        }
    }
}

fn reconstruct_r6(coarse: &CoarseField, m: usize) -> (Vec<f32>, Vec<f32>) {
    let models = precompute_r6_models(coarse, m);
    reconstruct_from_r6_models(coarse, &models, m)
}

// ---------------------------------------------------------------------------------------------
// R7 (round 4): R6 + a continuous coverage cap that saturates toward FULL coverage away from any
// material/empty frontier. The suspected mechanism behind the user's "quilting" verdict on R6: its
// `f = h0/h_ref` (`h_ref` = max over the inside 3x3, same as R4/R5) shrinks EVERY cell whose
// neighbourhood isn't perfectly flat, including a cell deep inside a body of material on a sand
// slope or a draining pool surface, where the uphill neighbour is simply higher than `h0` -- no
// empty space is nearby at all. That produces `f < 1` -- a shrunken tile -- at every such cell,
// which is exactly what a grid of visible seams ("quilting") looks like. `f_eff` reverts to full
// coverage in that case and keeps R6's own `f` at a real frontier, continuously and without a
// branch on material/wetness: see `cell_emptiness` for why a smooth slope's small per-cell height
// step saturates `emptiness` near 0 while a genuine empty neighbour saturates it near 1.
// ---------------------------------------------------------------------------------------------

/// Round 5: `emptiness = clamp(k * (1 - h_min/max(h0,eps)), 0, 1)` for a sharpening factor `k`
/// (round 4 shipped `k=1`; see `run_r7_k_sweep` for why a larger `k` was measured and the doc's
/// §10 for the result). `h_min` is "the minimum height over this cell's own INSIDE 3x3
/// neighbours, self excluded" per the original round-4 spec -- but note `neighbour_min_max`'s
/// `mn` (self-INCLUDED-by-initialisation) is mathematically identical for this purpose: whenever
/// the true neighbour minimum is >= h0 (flat/uphill/local-min), `mn` collapses to `h0` giving raw
/// `0`, while the self-excluded form gives some value `<= 0` that the caller's `clamp(_, 0, 1)`
/// also flattens to `0` -- same result either way, for every `k`. So this reuses the `mn` already
/// computed by `neighbour_min_max` (no second stencil pass), which is also exactly what the
/// shader's `eval_sub_cell_r7` does (it already has `mn` on hand from the Barth-Jespersen stencil).
/// 0 when every neighbour is at least as tall as `h0` (a flat interior, a local trough, or a normal
/// downhill slope step, where the neighbour is only slightly shorter than `h0` so the ratio stays
/// close to 1); saturates toward 1 (faster, for larger `k`) only when a neighbour is genuinely
/// close to empty (near 0) relative to `h0`.
fn r7_emptiness(mn: f32, h0: f32, k: f32) -> f32 {
    let raw = 1.0 - mn / h0.max(1e-6);
    (k * raw).clamp(0.0, 1.0)
}

/// Identical to `precompute_r6_models` except the strip width/offset are built from `f_eff =
/// mix(1.0, f, emptiness)` instead of R6's raw `f` -- confinement, bias, `w`, the plane, and the
/// closed-form conservation solve are all otherwise unchanged, so R7 reuses `R6Model`/`eval_r6`
/// verbatim. Conservation is unaffected by construction: the `delta` solve targets whatever
/// coverage field is actually used (built from `f_eff` here), exactly as it targets R6's `f`.
fn precompute_r7_models(coarse: &CoarseField, m: usize, k: f32) -> Vec<Option<R6Model>> {
    let n = coarse.size;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                out.push(None);
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            let (mn, mx) = neighbour_min_max(coarse, cx, cy);
            let h_ref = mx.max(1e-6);
            let f = (h0 / h_ref).clamp(0.0, 1.0);
            let emptiness = r7_emptiness(mn, h0, k);
            let f_eff = 1.0 + (f - 1.0) * emptiness; // mix(1.0, f, emptiness)
            let (gx, gy) = ls_gradient(coarse, cx, cy);
            let phi = bj_phi(h0, gx, gy, mn, mx);
            let (conf_x, bias_x) = axis_confinement_bias(coarse, cx, cy, 1, 0, h0, h_ref);
            let (conf_y, bias_y) = axis_confinement_bias(coarse, cx, cy, 0, 1, h0, h_ref);
            let wsum = conf_x + conf_y;
            let w = if wsum > 1e-6 { conf_x / wsum } else { 0.5 };
            let strip_half = f_eff / 2.0;
            let off_x = bias_x * (1.0 - f_eff) / 2.0;
            let off_y = bias_y * (1.0 - f_eff) / 2.0;
            let half = 0.5 / m as f32;
            let (mut sum_cov, mut sum_cov_h) = (0.0f32, 0.0f32);
            for j in 0..m {
                for i in 0..m {
                    let dx = (i as f32 + 0.5) / m as f32 - 0.5;
                    let dy = (j as f32 + 0.5) / m as f32 - 0.5;
                    let cov_x = overlap_1d(off_x, strip_half, dx, half);
                    let cov_y = overlap_1d(off_y, strip_half, dy, half);
                    let cov = w * cov_x + (1.0 - w) * cov_y;
                    let plane_h = h0 + phi * gx * dx + phi * gy * dy;
                    sum_cov += cov;
                    sum_cov_h += cov * plane_h;
                }
            }
            let mean_cov = sum_cov / (m * m) as f32;
            let mean_cov_h = sum_cov_h / (m * m) as f32;
            let delta = if mean_cov > 1e-6 { (h0 - mean_cov_h) / mean_cov } else { 0.0 };
            out.push(Some(R6Model { h0, gx, gy, phi, delta, off_x, off_y, strip_half, w }));
        }
    }
    out
}

/// Shared by `reconstruct_r6`/`reconstruct_r7`/the round-5 `k` sweep: evaluates a precomputed
/// `R6Model` field (from any of `precompute_r6_models`/`precompute_r7_models`) over the full fine
/// grid. Pulled out once both R6 and R7 needed it, plus the sweep needing it at arbitrary `k`.
fn reconstruct_from_r6_models(coarse: &CoarseField, models: &[Option<R6Model>], m: usize) -> (Vec<f32>, Vec<f32>) {
    let n = coarse.size * m;
    let mut h_out = vec![0.0f32; n * n];
    let mut cov_out = vec![0.0f32; n * n];
    for fy in 0..n {
        let yc = fine_to_coarse(fy, m);
        let cy = clamp_idx(yc.round() as i32, coarse.size);
        let dy = yc - cy as f32;
        for fx in 0..n {
            let xc = fine_to_coarse(fx, m);
            let cx = clamp_idx(xc.round() as i32, coarse.size);
            let dx = xc - cx as f32;
            let (hv, cv) = eval_r6(&models[cy * coarse.size + cx], dx, dy, m);
            h_out[fy * n + fx] = hv;
            cov_out[fy * n + fx] = cv;
        }
    }
    (h_out, cov_out)
}

fn reconstruct_r7(coarse: &CoarseField, m: usize, k: f32) -> (Vec<f32>, Vec<f32>) {
    let models = precompute_r7_models(coarse, m, k);
    reconstruct_from_r6_models(coarse, &models, m)
}

// ---------------------------------------------------------------------------------------------
// Interior/frontier false-negative split (round 4): the direct test of the R7 hypothesis. A coarse
// cell is "material-interior" if none of its INSIDE 3x3 neighbours is empty (height < THRESH) --
// i.e. no material/empty frontier passes through this cell's own neighbourhood, even if the
// neighbourhood isn't flat (a normal slope step still counts as interior here). OUTSIDE (wall)
// neighbours are excluded per the "walls are not empty" invariant, matching `h_ref`/`ls_gradient`.
// A FALSE NEGATIVE fine pixel (original >= THRESH, reconstruction not covered) is classified by
// which of these two buckets its own coarse cell falls into.
// ---------------------------------------------------------------------------------------------

fn material_interior_mask(coarse: &CoarseField, thresh: f32) -> Vec<bool> {
    let n = coarse.size;
    let mut out = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let mut interior = true;
            for ddy in -1..=1i32 {
                for ddx in -1..=1i32 {
                    if ddx == 0 && ddy == 0 {
                        continue;
                    }
                    let (nx, ny) = (cx + ddx, cy + ddy);
                    if coarse.inside(nx, ny) && coarse.get(nx, ny) < thresh {
                        interior = false;
                    }
                }
            }
            out[y * n + x] = interior;
        }
    }
    out
}

struct FnSplit { interior: usize, frontier: usize }

fn fn_interior_frontier_split(
    original: &[f32],
    cov_arr: &[f32],
    cov_thresh: f32,
    fine_mask: &[u8],
    fine_size: usize,
    material_interior: &[bool],
    m: usize,
    coarse_size: usize,
) -> FnSplit {
    let (mut interior, mut frontier) = (0usize, 0usize);
    for fy in 0..fine_size {
        let cy = (fy / m).min(coarse_size - 1);
        for fx in 0..fine_size {
            let idx = fy * fine_size + fx;
            if fine_mask[idx] == MASK_OUTSIDE {
                continue;
            }
            let cx = (fx / m).min(coarse_size - 1);
            let o = original[idx] >= THRESH;
            let r = cov_arr[idx] >= cov_thresh;
            if o && !r {
                if material_interior[cy * coarse_size + cx] {
                    interior += 1;
                } else {
                    frontier += 1;
                }
            }
        }
    }
    FnSplit { interior, frontier }
}

/// `(interior_mean_deficit, interior_n, frontier_mean_deficit, frontier_n)` -- mean `1-coverage`
/// over EVERY inside fine pixel (not just ones that cross a threshold), split by the same
/// material-interior/frontier classification as `fn_interior_frontier_split`. This is the
/// continuous companion to that binary split: a cell can shrink (coverage 0.7, say) without ever
/// crossing THRESH and being counted as a false negative, which is exactly the "shrunken tile"
/// mechanism the quilting hypothesis describes.
fn coverage_deficit_stats(cov: &[f32], fine_mask: &[u8], material_interior: &[bool], fine_size: usize, m: usize, coarse_size: usize) -> (f64, usize, f64, usize) {
    let (mut sum_i, mut n_i, mut sum_f, mut n_f) = (0.0f64, 0usize, 0.0f64, 0usize);
    for fy in 0..fine_size {
        let cy = (fy / m).min(coarse_size - 1);
        for fx in 0..fine_size {
            let idx = fy * fine_size + fx;
            if fine_mask[idx] == MASK_OUTSIDE {
                continue;
            }
            let cx = (fx / m).min(coarse_size - 1);
            let deficit = (1.0 - cov[idx] as f64).max(0.0);
            if material_interior[cy * coarse_size + cx] {
                sum_i += deficit;
                n_i += 1;
            } else {
                sum_f += deficit;
                n_f += 1;
            }
        }
    }
    (if n_i > 0 { sum_i / n_i as f64 } else { 0.0 }, n_i, if n_f > 0 { sum_f / n_f as f64 } else { 0.0 }, n_f)
}

// ---------------------------------------------------------------------------------------------
// Round 5, "also, briefly": two candidate SECOND mechanisms for a regular grid-pattern artifact at
// `m > 1`, independent of R7's `f_eff`/`k` entirely -- neither reads `f` or `emptiness` at all, so
// if either shows real structure, no choice of `k` in §step-1 can fix it.
// ---------------------------------------------------------------------------------------------

struct WInstability { mean_abs_dw: f64, n_pairs: usize, frac_extreme: f64, n_interior: usize }

/// Does R6/R7's per-axis blend weight `w = conf_x/(conf_x+conf_y)` flip abruptly between
/// neighbouring material-interior coarse cells even when neither axis shows a real feature (both
/// confinements near 0, so in principle `w` is deciding between two strips that both cover nearly
/// the whole cell and shouldn't matter -- but `w` itself is still computed as a ratio of two
/// noise-floor quantities there, and a ratio of near-zero numbers is exactly where a ratio is least
/// stable). `mean_abs_dw` is the mean `|w(neighbour)-w(cell)|` over every adjacent (4-connected)
/// pair of material-interior cells; `frac_extreme` is the fraction of material-interior cells
/// where `w` has already saturated near 0 or 1 (a near-binary axis pick) despite no axis showing a
/// real feature by construction (that's what "material-interior" means here). `w` does not depend
/// on `h0`'s relation to `h_ref` or on any `k` -- only on the 3x3 neighbour heights via
/// `axis_confinement_bias` -- so this is unaffected by anything in step 1.
fn w_instability_stats(coarse: &CoarseField, material_interior: &[bool]) -> WInstability {
    let n = coarse.size;
    let mut w_field = vec![f32::NAN; n * n];
    let (mut n_interior, mut n_extreme) = (0usize, 0usize);
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE || !material_interior[y * n + x] {
                continue;
            }
            let (cx, cy) = (x as i32, y as i32);
            let h0 = coarse.get(cx, cy);
            let (_, h_ref) = neighbour_min_max(coarse, cx, cy);
            let h_ref = h_ref.max(1e-6);
            let (conf_x, _) = axis_confinement_bias(coarse, cx, cy, 1, 0, h0, h_ref);
            let (conf_y, _) = axis_confinement_bias(coarse, cx, cy, 0, 1, h0, h_ref);
            let wsum = conf_x + conf_y;
            let w = if wsum > 1e-6 { conf_x / wsum } else { 0.5 };
            w_field[y * n + x] = w;
            n_interior += 1;
            if w < 0.1 || w > 0.9 {
                n_extreme += 1;
            }
        }
    }
    let (mut sum_dw, mut n_pairs) = (0.0f64, 0usize);
    for y in 0..n {
        for x in 0..n {
            let w0 = w_field[y * n + x];
            if w0.is_nan() {
                continue;
            }
            if x + 1 < n {
                let w1 = w_field[y * n + x + 1];
                if !w1.is_nan() {
                    sum_dw += (w1 - w0).abs() as f64;
                    n_pairs += 1;
                }
            }
            if y + 1 < n {
                let w1 = w_field[(y + 1) * n + x];
                if !w1.is_nan() {
                    sum_dw += (w1 - w0).abs() as f64;
                    n_pairs += 1;
                }
            }
        }
    }
    WInstability {
        mean_abs_dw: if n_pairs > 0 { sum_dw / n_pairs as f64 } else { 0.0 },
        n_pairs,
        frac_extreme: if n_interior > 0 { n_extreme as f64 / n_interior as f64 } else { 0.0 },
        n_interior,
    }
}

struct BoundaryJump { boundary_mean: f64, n_boundary: usize, within_mean: f64, n_within: usize }

/// Does the per-cell INDEPENDENTLY-FIT plane introduce a periodic discontinuity exactly at
/// coarse-cell boundaries -- which would look like a regular `m x m` grid of seams -- that isn't
/// present in the original field? Compares the mean absolute height jump between adjacent fine
/// pixels that CROSS a coarse-cell boundary against the same for pixel pairs that stay WITHIN one
/// coarse cell, both restricted to pairs where both participating coarse cells are
/// material-interior (i.e. no real edge should be present at all, in EITHER cell). If a rule's
/// boundary/within ratio sits far above the ORIGINAL field's own ratio (which has no reason to
/// know the coarse grid exists), that is a real, distinct artifact from coverage shrinkage -- a
/// property of the independently-fit plane, not of `f`/`emptiness`, so no choice of R7's `k` can
/// move it.
fn boundary_jump_stats(recon: &[f32], fine_mask: &[u8], material_interior: &[bool], fine_size: usize, m: usize, coarse_size: usize) -> BoundaryJump {
    let (mut sum_b, mut n_b, mut sum_w, mut n_w) = (0.0f64, 0usize, 0.0f64, 0usize);
    for fy in 0..fine_size {
        let cy = (fy / m).min(coarse_size - 1);
        for fx in 1..fine_size {
            let (idx, idx0) = (fy * fine_size + fx, fy * fine_size + fx - 1);
            if fine_mask[idx] == MASK_OUTSIDE || fine_mask[idx0] == MASK_OUTSIDE {
                continue;
            }
            let cx = (fx / m).min(coarse_size - 1);
            let cx0 = ((fx - 1) / m).min(coarse_size - 1);
            if !material_interior[cy * coarse_size + cx] || !material_interior[cy * coarse_size + cx0] {
                continue;
            }
            let jump = (recon[idx] - recon[idx0]).abs() as f64;
            if fx % m == 0 {
                sum_b += jump;
                n_b += 1;
            } else {
                sum_w += jump;
                n_w += 1;
            }
        }
    }
    for fy in 1..fine_size {
        for fx in 0..fine_size {
            let (idx, idx0) = (fy * fine_size + fx, (fy - 1) * fine_size + fx);
            if fine_mask[idx] == MASK_OUTSIDE || fine_mask[idx0] == MASK_OUTSIDE {
                continue;
            }
            let cy = (fy / m).min(coarse_size - 1);
            let cy0 = ((fy - 1) / m).min(coarse_size - 1);
            let cx = (fx / m).min(coarse_size - 1);
            if !material_interior[cy * coarse_size + cx] || !material_interior[cy0 * coarse_size + cx] {
                continue;
            }
            let jump = (recon[idx] - recon[idx0]).abs() as f64;
            if fy % m == 0 {
                sum_b += jump;
                n_b += 1;
            } else {
                sum_w += jump;
                n_w += 1;
            }
        }
    }
    BoundaryJump {
        boundary_mean: if n_b > 0 { sum_b / n_b as f64 } else { 0.0 },
        n_boundary: n_b,
        within_mean: if n_w > 0 { sum_w / n_w as f64 } else { 0.0 },
        n_within: n_w,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule { R0, R1, R2, R3, R4, R5, R6, R7 }

impl Rule {
    fn name(&self) -> &'static str {
        match self {
            Rule::R0 => "R0_nearest",
            Rule::R1 => "R1_bilinear",
            Rule::R2 => "R2_limited_plane",
            Rule::R3 => "R3_face_match",
            Rule::R4 => "R4_plic",
            Rule::R5 => "R5_plic_aa",
            Rule::R6 => "R6_strip_aa",
            Rule::R7 => "R7_strip_sat",
        }
    }
}

/// Reconstructs the full `n x n` (`n = coarse.size * m`) fine field for one rule.
fn reconstruct(coarse: &CoarseField, rule: Rule, m: usize) -> Vec<f32> {
    let n = coarse.size * m;
    let mut out = vec![0.0f32; n * n];
    match rule {
        Rule::R0 | Rule::R1 => {
            for fy in 0..n {
                let yc = fine_to_coarse(fy, m);
                for fx in 0..n {
                    let xc = fine_to_coarse(fx, m);
                    out[fy * n + fx] = if rule == Rule::R0 { r0_sample(coarse, xc, yc) } else { r1_sample(coarse, xc, yc) };
                }
            }
        }
        Rule::R2 | Rule::R3 => {
            let models = precompute_plane_models(coarse, m, rule == Rule::R3);
            fill_from_models(&mut out, coarse, &models, m);
        }
        Rule::R4 => {
            let models = precompute_plic_models(coarse);
            fill_from_models(&mut out, coarse, &models, m);
        }
        Rule::R5 | Rule::R6 | Rule::R7 => unreachable!("R5/R6/R7 have their own coverage output -- call reconstruct_pair instead"),
    }
    out
}

/// The single entry point every metric/picture call site should use: returns
/// `(height_for_rms_mass_width, coverage_comparable_array, coverage_threshold)`. For R0-R4 the
/// coverage-comparable array IS the height array and the threshold is the shipped `THRESH`
/// (0.003) -- i.e. "covered" means "height at or above the shader's opacity cutoff", exactly as
/// today.
///
/// For R5/R6/R7, round 3 fixed a real defect in round 2's convention: thresholding the raw
/// coverage fraction alone at 0.5 let a nearly-empty film-case cell (all neighbours shallow, so
/// `f=h0/h_ref` is close to 1 even though `h0` itself is tiny) read as "fully covered" and draw a
/// visible fleck at near-zero height. The shader's own opacity already depends on HEIGHT
/// (`empty_blend = clamp(h/0.003, 0, 1)`), so "covered" for these rules now requires BOTH
/// `coverage >= 0.5` AND the coverage-weighted height clearing the same 0.003 the other rules
/// use: encoded here as one array/threshold pair by writing a sentinel (`-1.0`, which can never
/// clear any positive threshold) wherever `coverage < 0.5`, and the coverage-weighted height
/// (`h`, already `coverage*plane_height`) everywhere else -- so `coverage_metrics`/`boundary_mask`
/// thresholding this array at `THRESH` reproduces the AND exactly, with no change to either
/// generic function.
fn reconstruct_pair(coarse: &CoarseField, rule: Rule, m: usize) -> (Vec<f32>, Vec<f32>, f32) {
    if rule == Rule::R5 || rule == Rule::R6 || rule == Rule::R7 {
        let (h, cov) = match rule {
            Rule::R5 => reconstruct_r5(coarse, m),
            Rule::R6 => reconstruct_r6(coarse, m),
            Rule::R7 => reconstruct_r7(coarse, m, R7_K),
            _ => unreachable!(),
        };
        let combined: Vec<f32> = (0..h.len()).map(|i| if cov[i] >= 0.5 { h[i] } else { -1.0 }).collect();
        (h, combined, THRESH)
    } else {
        let h = reconstruct(coarse, rule, m);
        (h.clone(), h, THRESH)
    }
}

fn fill_from_models(out: &mut [f32], coarse: &CoarseField, models: &[Option<CellModel>], m: usize) {
    let n = coarse.size * m;
    for fy in 0..n {
        let yc = fine_to_coarse(fy, m);
        let cy = clamp_idx(yc.round() as i32, coarse.size);
        let dy = yc - cy as f32;
        for fx in 0..n {
            let xc = fine_to_coarse(fx, m);
            let cx = clamp_idx(xc.round() as i32, coarse.size);
            let dx = xc - cx as f32;
            out[fy * n + fx] = eval_model(&models[cy * coarse.size + cx], dx, dy);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------------------------

fn max_mass_error(coarse: &CoarseField, recon: &[f32], m: usize) -> f32 {
    let n = coarse.size * m;
    let mut worst = 0.0f32;
    for cy in 0..coarse.size {
        for cx in 0..coarse.size {
            if coarse.mask[cy * coarse.size + cx] == MASK_OUTSIDE {
                continue;
            }
            let mut sum = 0.0f32;
            for dy in 0..m {
                for dx in 0..m {
                    sum += recon[(cy * m + dy) * n + (cx * m + dx)];
                }
            }
            let mean = sum / (m * m) as f32;
            worst = worst.max((mean - coarse.h[cy * coarse.size + cx]).abs());
        }
    }
    worst
}

/// A fine cell counts as "coarse-interior" if the coarse cell it belongs to has all 8 of its
/// own 3x3 coarse neighbours non-OUTSIDE (fully surrounded, no wall/void nearby at THIS
/// resolution); otherwise "frontier".
fn coarse_interior_mask(coarse: &CoarseField) -> Vec<bool> {
    let n = coarse.size;
    let mut out = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            if coarse.mask[y * n + x] == MASK_OUTSIDE {
                continue;
            }
            let mut all_in = true;
            for ddy in -1..=1i32 {
                for ddx in -1..=1i32 {
                    if !coarse.inside(x as i32 + ddx, y as i32 + ddy) {
                        all_in = false;
                    }
                }
            }
            out[y * n + x] = all_in;
        }
    }
    out
}

struct RmsResult { all: f64, interior: f64, frontier: f64 }

fn height_rms(original: &[f32], recon: &[f32], fine_mask: &[u8], fine_size: usize, coarse_interior: &[bool], m: usize, coarse_size: usize) -> RmsResult {
    let (mut se_i, mut se_f) = (0.0f64, 0.0f64);
    let (mut n_i, mut n_f) = (0usize, 0usize);
    for fy in 0..fine_size {
        let cy = (fy / m).min(coarse_size - 1);
        for fx in 0..fine_size {
            let idx = fy * fine_size + fx;
            if fine_mask[idx] == MASK_OUTSIDE {
                continue;
            }
            let cx = (fx / m).min(coarse_size - 1);
            let e = (recon[idx] - original[idx]) as f64;
            if coarse_interior[cy * coarse_size + cx] {
                se_i += e * e;
                n_i += 1;
            } else {
                se_f += e * e;
                n_f += 1;
            }
        }
    }
    let all = if n_i + n_f > 0 { ((se_i + se_f) / (n_i + n_f) as f64).sqrt() } else { 0.0 };
    RmsResult {
        all,
        interior: if n_i > 0 { (se_i / n_i as f64).sqrt() } else { 0.0 },
        frontier: if n_f > 0 { (se_f / n_f as f64).sqrt() } else { 0.0 },
    }
}

struct Coverage { fp: usize, fn_: usize, iou: f64 }

fn coverage_metrics(original: &[f32], recon: &[f32], fine_mask: &[u8], size: usize, thresh: f32) -> Coverage {
    let (mut fp, mut fn_, mut tp, mut union) = (0usize, 0usize, 0usize, 0usize);
    for i in 0..size * size {
        if fine_mask[i] == MASK_OUTSIDE {
            continue;
        }
        let o = original[i] >= thresh;
        let r = recon[i] >= thresh;
        if o || r {
            union += 1;
        }
        if o && r {
            tp += 1;
        } else if r && !o {
            fp += 1;
        } else if o && !r {
            fn_ += 1;
        }
    }
    Coverage { fp, fn_, iou: if union > 0 { tp as f64 / union as f64 } else { 1.0 } }
}

/// Rows where the vessel's inside-cell count is a local minimum relative to a +/-5 row window
/// and below 40% of the widest row -- generic neck detector, works for any hourglass-family mask.
fn find_neck_rows(mask: &[u8], size: usize) -> Vec<usize> {
    let counts: Vec<usize> = (0..size).map(|y| (0..size).filter(|&x| mask[y * size + x] != MASK_OUTSIDE).count()).collect();
    let max_count = *counts.iter().max().unwrap_or(&1);
    let win = 5usize;
    let mut necks = Vec::new();
    for y in win..size.saturating_sub(win) {
        let c = counts[y];
        if c == 0 || (c as f32) >= 0.4 * max_count as f32 {
            continue;
        }
        if counts[y - win..y].iter().all(|&v| v >= c) && counts[y + 1..=y + win].iter().all(|&v| v >= c) {
            necks.push(y);
        }
    }
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for y in necks {
        if let Some(last) = clusters.last_mut() {
            if y as i64 - *last.last().unwrap() as i64 <= 10 {
                last.push(y);
                continue;
            }
        }
        clusters.push(vec![y]);
    }
    clusters.iter().map(|c| c[c.len() / 2]).collect()
}

// ---------------------------------------------------------------------------------------------
// Round-2 fix #1: PER-STREAM width. The round-1 metric summed occupied/mass over an entire row,
// which for a multi-neck vessel measures the SEPARATION between streams as much as any single
// stream's width. This detects each stream as its own contiguous covered span, matches spans
// between the original and a candidate by nearest centre, and reports width ratios per matched
// stream -- averaged over streams and over three rows chosen to be in FREE FALL (round-1 probed
// only `neck_row+2`, which is still inside the neck's own throat, not free fall).
// ---------------------------------------------------------------------------------------------

/// Contiguous covered (`h[idx] >= thresh` and inside-mask) runs along one row -- one entry per
/// stream (or per lobe of a pool's covered surface, off a stream context).
fn detect_spans(h: &[f32], mask: &[u8], size: usize, row: usize, thresh: f32) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for x in 0..size {
        let idx = row * size + x;
        let covered = mask[idx] != MASK_OUTSIDE && h[idx] >= thresh;
        match (covered, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => { spans.push((s, x)); start = None; }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, size));
    }
    spans
}

struct SpanMetrics { occupied_px: usize, mass_equiv: f64 }

fn span_metrics(h: &[f32], mask: &[u8], size: usize, row: usize, span: (usize, usize)) -> SpanMetrics {
    let (a, b) = span;
    let (mut mass, mut peak) = (0.0f64, 0.0f32);
    for x in a..b {
        let idx = row * size + x;
        if mask[idx] == MASK_OUTSIDE {
            continue;
        }
        mass += h[idx] as f64;
        peak = peak.max(h[idx]);
    }
    SpanMetrics { occupied_px: b - a, mass_equiv: if peak > 0.0 { mass / peak as f64 } else { 0.0 } }
}

/// Greedy nearest-centre matching between the original's spans and a candidate's spans at the
/// same row. Unmatched originals ("missing" -- the rule dropped or merged a stream) and unmatched
/// candidate spans ("extra" -- the rule split one stream into two, or hallucinated one) are
/// reported explicitly rather than silently skipped or silently averaged in.
fn match_spans(orig: &[(usize, usize)], rule: &[(usize, usize)]) -> (Vec<((usize, usize), (usize, usize))>, usize, usize) {
    let center = |s: &(usize, usize)| (s.0 + s.1) as f32 / 2.0;
    let mut used = vec![false; rule.len()];
    let mut pairs = Vec::new();
    let mut missing = 0usize;
    for o in orig {
        let oc = center(o);
        let mut best: Option<(usize, f32)> = None;
        for (i, r) in rule.iter().enumerate() {
            if used[i] {
                continue;
            }
            let d = (center(r) - oc).abs();
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        match best {
            Some((i, _)) => { used[i] = true; pairs.push((*o, rule[i])); }
            None => missing += 1,
        }
    }
    let extra = used.iter().filter(|u| !**u).count();
    (pairs, missing, extra)
}

struct StreamAgg { n_matched: usize, n_missing: usize, n_extra: usize, mean_occ_ratio: f64, mean_mass_ratio: f64 }

/// Averages per-stream width/mass ratios (candidate/original) over every matched stream at every
/// row in `rows`. `thresh` is the coverage threshold used to detect spans on BOTH fields -- always
/// `THRESH` on a height array for R0-R4, but the caller passes R5's own coverage-weighted height
/// array here too (not the raw coverage fraction): per the round-2 brief, R5's width/mass metrics
/// use coverage*height, so span detection on that product at the normal 0.003 cutoff is exactly
/// the intended quantity.
fn per_stream_width(orig_h: &[f32], recon_h: &[f32], mask: &[u8], size: usize, rows: &[usize], thresh: f32) -> StreamAgg {
    let (mut occ_ratios, mut mass_ratios) = (Vec::new(), Vec::new());
    let (mut matched, mut missing, mut extra) = (0usize, 0usize, 0usize);
    for &row in rows {
        let orig_spans = detect_spans(orig_h, mask, size, row, thresh);
        let rule_spans = detect_spans(recon_h, mask, size, row, thresh);
        let (pairs, miss, ext) = match_spans(&orig_spans, &rule_spans);
        missing += miss;
        extra += ext;
        for (o, r) in pairs {
            let om = span_metrics(orig_h, mask, size, row, o);
            let rm = span_metrics(recon_h, mask, size, row, r);
            matched += 1;
            if om.occupied_px > 0 {
                occ_ratios.push(rm.occupied_px as f64 / om.occupied_px as f64);
            }
            if om.mass_equiv > 1e-9 {
                mass_ratios.push(rm.mass_equiv / om.mass_equiv);
            }
        }
    }
    let mean = |v: &[f64]| if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 };
    StreamAgg { n_matched: matched, n_missing: missing, n_extra: extra, mean_occ_ratio: mean(&occ_ratios), mean_mass_ratio: mean(&mass_ratios) }
}

// ---------------------------------------------------------------------------------------------
// Round-2 fix #2: front fidelity via a symmetric chamfer distance, since a global fp/fn/IoU count
// is dominated by pool/wall area and is blind to staircasing along a front (the actual complaint
// -- "curves not smooth"). A 2-pass chamfer-(1, sqrt2) distance transform approximates Euclidean
// distance to within ~2%, plenty for the px-scale distances measured here.
// ---------------------------------------------------------------------------------------------

fn boundary_mask(h: &[f32], mask: &[u8], size: usize, thresh: f32) -> Vec<bool> {
    let mut b = vec![false; size * size];
    for y in 0..size {
        for x in 0..size {
            let idx = y * size + x;
            if mask[idx] == MASK_OUTSIDE {
                continue;
            }
            let covered = h[idx] >= thresh;
            let mut edge = false;
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx as usize >= size || ny as usize >= size {
                    continue;
                }
                let nidx = ny as usize * size + nx as usize;
                let n_covered = mask[nidx] != MASK_OUTSIDE && h[nidx] >= thresh;
                if n_covered != covered {
                    edge = true;
                }
            }
            b[idx] = edge;
        }
    }
    b
}

const CHAMFER_INF: f32 = 1e9;

fn chamfer_dt(boundary: &[bool], size: usize) -> Vec<f32> {
    let mut d = vec![CHAMFER_INF; size * size];
    for (i, &b) in boundary.iter().enumerate() {
        if b {
            d[i] = 0.0;
        }
    }
    let s2 = std::f32::consts::SQRT_2;
    for y in 0..size {
        for x in 0..size {
            let idx = y * size + x;
            let mut best = d[idx];
            if x > 0 { best = best.min(d[idx - 1] + 1.0); }
            if y > 0 { best = best.min(d[idx - size] + 1.0); }
            if x > 0 && y > 0 { best = best.min(d[idx - size - 1] + s2); }
            if x + 1 < size && y > 0 { best = best.min(d[idx - size + 1] + s2); }
            d[idx] = best;
        }
    }
    for y in (0..size).rev() {
        for x in (0..size).rev() {
            let idx = y * size + x;
            let mut best = d[idx];
            if x + 1 < size { best = best.min(d[idx + 1] + 1.0); }
            if y + 1 < size { best = best.min(d[idx + size] + 1.0); }
            if x + 1 < size && y + 1 < size { best = best.min(d[idx + size + 1] + s2); }
            if x > 0 && y + 1 < size { best = best.min(d[idx + size - 1] + s2); }
            d[idx] = best;
        }
    }
    d
}

struct ChamferResult { fwd_mean: f64, fwd_max: f32, rev_mean: f64, rev_max: f32, n_fwd: usize, n_rev: usize }

/// Symmetric chamfer over a rectangular region only (a slope front / a pool-wall edge / the sides
/// of a stream), not the whole grid -- fp/fn/IoU are already global and dominated by bulk pool and
/// wall area; this is deliberately local to the feature being judged. `fwd` = distance from each
/// RECONSTRUCTED boundary pixel in the region to the nearest ORIGINAL boundary pixel (using the
/// original's precomputed distance transform); `rev` = the reverse. Staircasing shows up as
/// `mean ~= m/4`, `max ~= m/2` in whichever direction has the staircase.
fn front_fidelity(orig_boundary: &[bool], orig_dt: &[f32], rule_boundary: &[bool], rule_dt: &[f32], size: usize, region: (usize, usize, usize, usize)) -> ChamferResult {
    let (x0, y0, w, hh) = region;
    let (mut fwd, mut rev) = (Vec::new(), Vec::new());
    for y in y0..(y0 + hh).min(size) {
        for x in x0..(x0 + w).min(size) {
            let idx = y * size + x;
            if rule_boundary[idx] {
                fwd.push(orig_dt[idx] as f64);
            }
            if orig_boundary[idx] {
                rev.push(rule_dt[idx] as f64);
            }
        }
    }
    let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    let maxv = |v: &[f64]| v.iter().cloned().fold(0.0f64, f64::max) as f32;
    ChamferResult { fwd_mean: mean(&fwd), fwd_max: maxv(&fwd), rev_mean: mean(&rev), rev_max: maxv(&rev), n_fwd: fwd.len(), n_rev: rev.len() }
}

/// Finds a `box_size` square, fully inside the mask, in the bottom half of the grid, maximising
/// average height -- a settled pool/pile interior, used for the smoothness (2nd-difference) probe.
fn find_pool_region(mask: &[u8], h: &[f32], size: usize, box_size: usize) -> (usize, usize) {
    let mut best = (size / 2 - box_size / 2, size * 3 / 4);
    let mut best_score = -1.0f32;
    let mut y = size / 2;
    while y + box_size < size {
        let mut x = 0;
        while x + box_size < size {
            let mut all_inside = true;
            let mut sum = 0.0f32;
            'scan: for by in 0..box_size {
                for bx in 0..box_size {
                    let idx = (y + by) * size + (x + bx);
                    if mask[idx] == MASK_OUTSIDE {
                        all_inside = false;
                        break 'scan;
                    }
                    sum += h[idx];
                }
            }
            if all_inside {
                let avg = sum / (box_size * box_size) as f32;
                if avg > best_score {
                    best_score = avg;
                    best = (x, y);
                }
            }
            x += 4;
        }
        y += 4;
    }
    best
}

fn second_diff_rms(h: &[f32], size: usize, x0: usize, y0: usize, box_size: usize) -> f64 {
    let mut se = 0.0f64;
    let mut n = 0usize;
    for by in 1..box_size - 1 {
        for bx in 1..box_size - 1 {
            let (x, y) = (x0 + bx, y0 + by);
            let idx = |xx: usize, yy: usize| h[yy * size + xx] as f64;
            let d2x = idx(x + 1, y) - 2.0 * idx(x, y) + idx(x - 1, y);
            let d2y = idx(x, y + 1) - 2.0 * idx(x, y) + idx(x, y - 1);
            se += d2x * d2x + d2y * d2y;
            n += 1;
        }
    }
    if n > 0 { (se / n as f64).sqrt() } else { 0.0 }
}

/// Finds a `box_size` square, fully inside the mask (no vessel wall at all in the box, so what's
/// found is a genuine granular repose slope, not a diagonal container wall), in the bottom
/// two-fifths of the grid, maximising local height RANGE -- a pile's slope front.
fn find_slope_region(mask: &[u8], h: &[f32], size: usize, box_size: usize) -> (usize, usize) {
    let mut best = (size / 2 - box_size / 2, size / 2);
    let mut best_score = -1.0f32;
    let mut y = size * 3 / 5;
    while y + box_size < size {
        let mut x = 0;
        while x + box_size < size {
            let (mut mn, mut mx, mut inside_cnt) = (f32::MAX, f32::MIN, 0usize);
            for by in 0..box_size {
                for bx in 0..box_size {
                    let idx = (y + by) * size + (x + bx);
                    if mask[idx] != MASK_OUTSIDE {
                        inside_cnt += 1;
                        mn = mn.min(h[idx]);
                        mx = mx.max(h[idx]);
                    }
                }
            }
            if inside_cnt == box_size * box_size {
                let score = mx - mn;
                if score > best_score {
                    best_score = score;
                    best = (x, y);
                }
            }
            x += 4;
        }
        y += 4;
    }
    best
}

// ---------------------------------------------------------------------------------------------
// Shared region finders -- used by BOTH the numeric front-fidelity probe (`run_snapshot`) and the
// picture crops (`make_all_pictures`), so the region a number is reported for is exactly the
// region shown.
// ---------------------------------------------------------------------------------------------

/// `(x0, y0, w, h)` around the first detected neck and the stream(s) below it.
fn stream_region(mask: &[u8], size: usize, box_size: usize) -> (usize, usize, usize, usize) {
    let neck_rows = find_neck_rows(mask, size);
    let neck_row = *neck_rows.first().unwrap_or(&(size / 3));
    (size / 2 - box_size / 2, neck_row.saturating_sub(10), box_size, box_size)
}

/// `(x0, y0, w, h)` around a settled pool/pile interior slid sideways to the nearest vessel wall,
/// so the crop/probe includes the wall boundary itself, not just the flat interior.
fn pool_wall_region(mask: &[u8], h: &[f32], size: usize, box_size: usize) -> (usize, usize, usize, usize) {
    let interior_box = box_size / 2;
    let (pool_x, pool_y) = find_pool_region(mask, h, size, interior_box);
    let mut wall_x = pool_x;
    for x in pool_x.saturating_sub(box_size)..(pool_x + interior_box + box_size).min(size) {
        if mask[pool_y * size + x] == MASK_OUTSIDE {
            wall_x = x;
            break;
        }
    }
    let x0 = wall_x.saturating_sub(box_size / 2).min(size.saturating_sub(box_size));
    (x0, pool_y.saturating_sub(box_size / 4), box_size, box_size)
}

/// `(x0, y0, w, h)` fully inside the mask (no wall) in the lower chamber, at the location of
/// greatest height range -- a granular repose slope or a settling liquid surface.
fn slope_region(mask: &[u8], h: &[f32], size: usize, box_size: usize) -> (usize, usize, usize, usize) {
    let (x, y) = find_slope_region(mask, h, size, box_size);
    (x, y, box_size, box_size)
}

// ---------------------------------------------------------------------------------------------
// PNG output
// ---------------------------------------------------------------------------------------------

/// `h` is the displayed height (grayscale); `cov`/`cov_thresh` is what decides the coverage
/// outline -- for R0-R4 this is the same array as `h` at `THRESH`, for R5 it's the raw coverage
/// fraction at 0.5 (round-2: R5's "covered" is a coverage-fraction question, not a height one).
fn crop_image(h: &[f32], cov: &[f32], cov_thresh: f32, mask: &[u8], size: usize, x0: usize, y0: usize, w: usize, hh: usize, vmax: f32) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, hh as u32);
    for by in 0..hh {
        for bx in 0..w {
            let (x, y) = (x0 + bx, y0 + by);
            let idx = y * size + x;
            let outside = mask[idx] == MASK_OUTSIDE;
            let v = (h[idx] / vmax).clamp(0.0, 1.0);
            let g = (v * 235.0) as u8 + if outside { 0 } else { 20 };
            let mut px = if outside { [30u8, 26u8, 22u8] } else { [g, g, g] };
            if !outside {
                let covered = cov[idx] >= cov_thresh;
                let mut edge = false;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx as usize >= size || ny as usize >= size {
                        continue;
                    }
                    let nidx = ny as usize * size + nx as usize;
                    let n_covered = mask[nidx] != MASK_OUTSIDE && cov[nidx] >= cov_thresh;
                    if n_covered != covered {
                        edge = true;
                    }
                }
                if edge {
                    px = [255, 120, 40];
                }
            }
            img.put_pixel(bx as u32, by as u32, image::Rgb(px));
        }
    }
    img
}

/// Round-2 fix #3: the 128px crops were too small to judge staircasing by eye. Plain nearest-
/// neighbour pixel replication (no new information, just legibility) up to `factor`x.
fn magnify_nearest(img: &image::RgbImage, factor: u32) -> image::RgbImage {
    let (w, h) = img.dimensions();
    let mut out = image::RgbImage::new(w * factor, h * factor);
    for y in 0..h {
        for x in 0..w {
            let p = *img.get_pixel(x, y);
            for dy in 0..factor {
                for dx in 0..factor {
                    out.put_pixel(x * factor + dx, y * factor + dy, p);
                }
            }
        }
    }
    out
}

fn make_contact_sheet(tiles: &[image::RgbImage], cols: usize) -> image::RgbImage {
    let cols = cols.min(tiles.len().max(1));
    let rows = (tiles.len() + cols - 1) / cols;
    let (tw, th) = tiles.first().map(|t| (t.width(), t.height())).unwrap_or((1, 1));
    let pad = 6u32;
    let mut sheet = image::RgbImage::from_pixel(cols as u32 * (tw + pad) + pad, rows as u32 * (th + pad) + pad, image::Rgb([245, 245, 245]));
    for (i, tile) in tiles.iter().enumerate() {
        let (col, row) = (i % cols, i / cols);
        let ox = pad + col as u32 * (tw + pad);
        let oy = pad + row as u32 * (th + pad);
        image::imageops::overlay(&mut sheet, tile, ox as i64, oy as i64);
    }
    sheet
}

// ---------------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------------

fn output_dir() -> std::path::PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().expect("sandart-sim has a workspace parent");
    let dir = root.join("artifacts/design/upscale-2026-09-14");
    std::fs::create_dir_all(&dir).expect("create output dir");
    dir
}

const ALL_RULES: [Rule; 8] = [Rule::R0, Rule::R1, Rule::R2, Rule::R3, Rule::R4, Rule::R5, Rule::R6, Rule::R7];

fn run_snapshot(snap: &Snapshot) {
    let size = snap.sim.heightmap.width;
    println!("\n================ snapshot: {} (S=512) ================", snap.name);

    // Regions + original boundary/distance-transform state that do NOT depend on m or rule --
    // computed once per snapshot, reused across the m loop below.
    let neck_rows_fine = find_neck_rows(&snap.mask512, size);
    println!("detected neck rows (fine, S=512): {neck_rows_fine:?}");
    if std::env::var("DUMP_ROW_COUNTS").is_ok() {
        let counts: Vec<usize> = (0..size).map(|y| (0..size).filter(|&x| snap.mask512[y * size + x] != MASK_OUTSIDE).count()).collect();
        for (y, c) in counts.iter().enumerate() {
            if y % 4 == 0 { println!("row {y}: {c}"); }
        }
    }
    let neck_row = *neck_rows_fine.first().unwrap_or(&(size / 3));
    // Round-2 fix: probe FREE FALL (16/32/48 cells below the neck), not `neck_row+2` (still
    // inside the neck's own throat).
    let free_fall_rows: Vec<usize> = [16usize, 32, 48].iter().filter_map(|&d| if neck_row + d < size { Some(neck_row + d) } else { None }).collect();

    let pool_box = 24usize;
    let (pool_x, pool_y) = find_pool_region(&snap.mask512, &snap.h, size, pool_box);
    let orig_smooth = second_diff_rms(&snap.h, size, pool_x, pool_y, pool_box);
    println!(
        "original pool-interior 2nd-diff RMS at ({pool_x},{pool_y}) box={pool_box}: {:.9} (near-zero is expected for a settled pool/pile plateau -- this is the TRUE baseline, not noise)",
        orig_smooth
    );

    let region_stream = stream_region(&snap.mask512, size, 128);
    let region_pool = pool_wall_region(&snap.mask512, &snap.h, size, 128);
    let region_slope = slope_region(&snap.mask512, &snap.h, size, 128);
    println!("front-fidelity regions: stream={region_stream:?} pool_wall={region_pool:?} slope={region_slope:?}");

    let orig_boundary = boundary_mask(&snap.h, &snap.mask512, size, THRESH);
    let orig_dt = chamfer_dt(&orig_boundary, size);

    for &m in &[2usize, 4usize] {
        let coarse_size = size / m;
        let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
        let (coarse, dreport) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
        let coarse_interior = coarse_interior_mask(&coarse);
        // Round 4: material-interior mask (independent of rule, built from the ORIGINAL coarse
        // field only) for the interior/frontier false-negative split below.
        let material_interior = material_interior_mask(&coarse, THRESH);

        let loss_pct = if dreport.total_mass > 0.0 { 100.0 * dreport.mass_lost / dreport.total_mass } else { 0.0 };
        println!(
            "\n-- m={m} (coarse S={coarse_size}) -- downscale mass lost to blocks outside the coarse mask: {:.6} / {:.6} total ({:.4}%)",
            dreport.mass_lost, dreport.total_mass, loss_pct
        );

        println!(
            "{:<18} {:>12} {:>10} {:>10} {:>10} {:>8} {:>8} {:>8} {:>12}",
            "rule", "max_mass_err", "rms_all", "rms_int", "rms_front", "fp_px", "fn_px", "iou", "smooth_abs"
        );

        for &rule in &ALL_RULES {
            let (recon, cov_arr, cov_thresh) = reconstruct_pair(&coarse, rule, m);
            let mass_err = max_mass_error(&coarse, &recon, m);
            let rms = height_rms(&snap.h, &recon, &snap.mask512, size, &coarse_interior, m, coarse_size);
            let cov = coverage_metrics(&snap.h, &cov_arr, &snap.mask512, size, cov_thresh);
            let recon_smooth = second_diff_rms(&recon, size, pool_x, pool_y, pool_box);

            println!(
                "{:<18} {:>12.6} {:>10.6} {:>10.6} {:>10.6} {:>8} {:>8} {:>8.4} {:>12.9}",
                rule.name(), mass_err, rms.all, rms.interior, rms.frontier, cov.fp, cov.fn_, cov.iou, recon_smooth
            );

            // Round 4: split false negatives by whether they sit in a coarse cell with a real
            // material/empty frontier nearby, or one whose neighbourhood is fully material (the
            // direct test of the "R6 shrinks interior cells too" hypothesis).
            if cov.fn_ > 0 {
                let split = fn_interior_frontier_split(&snap.h, &cov_arr, cov_thresh, &snap.mask512, size, &material_interior, m, coarse_size);
                println!(
                    "    fn_split: interior={} frontier={} (of fn_px={}; interior = coarse cell with no empty neighbour, i.e. NOT at a material/empty frontier)",
                    split.interior, split.frontier, cov.fn_
                );
            }

            // Round-2 fix A: per-stream width, free fall only, matched by span not by row sum.
            if !free_fall_rows.is_empty() {
                let agg = per_stream_width(&snap.h, &recon, &snap.mask512, size, &free_fall_rows, THRESH);
                println!(
                    "    per-stream width @ y={free_fall_rows:?}: matched={} missing={} extra={} occ_ratio_mean={:.3} mass_ratio_mean={:.3} (target 1.0 for both -- this is a ROUND-TRIP, not a resolution comparison)",
                    agg.n_matched, agg.n_missing, agg.n_extra, agg.mean_occ_ratio, agg.mean_mass_ratio
                );
            }

            // Round-2 fix B: front fidelity (symmetric chamfer), local to each named feature.
            let rule_boundary = boundary_mask(&cov_arr, &snap.mask512, size, cov_thresh);
            let rule_dt = chamfer_dt(&rule_boundary, size);
            for (label, region) in [("stream_sides", region_stream), ("pool_wall_edge", region_pool), ("slope_front", region_slope)] {
                let cf = front_fidelity(&orig_boundary, &orig_dt, &rule_boundary, &rule_dt, size, region);
                println!(
                    "    front_fidelity[{label}] fwd(recon->orig) mean={:.3} max={:.3} n={} | rev(orig->recon) mean={:.3} max={:.3} n={} (px; m/4={:.2}, m/2={:.2})",
                    cf.fwd_mean, cf.fwd_max, cf.n_fwd, cf.rev_mean, cf.rev_max, cf.n_rev, m as f64 / 4.0, m as f64 / 2.0
                );
            }
        }
        println!("(rms/mass units are heightmap units 0..1; pixel counts are over the full 512x512 fine-inside domain)");

        // Round 4, continuous check: the binary fn_split above only counts a cell where coverage
        // drops the reconstructed height below THRESH -- it is blind to a cell that merely shrinks
        // (coverage < 1 but still comfortably above THRESH), which is what "quilting" actually
        // looks like at most interior cells on a gentle slope. This measures mean coverage DEFICIT
        // (1 - coverage) over every inside fine pixel, split the same way, for the raw coverage
        // fields of R5/R6/R7 (R4's coverage is binary in/out, not a fraction, so this doesn't apply
        // to it in the same sense).
        println!("    -- mean coverage deficit (1-coverage), continuous, not threshold-gated --");
        for (name, cov) in [("R5_plic_aa", reconstruct_r5(&coarse, m).1), ("R6_strip_aa", reconstruct_r6(&coarse, m).1), ("R7_strip_sat", reconstruct_r7(&coarse, m, R7_K).1)] {
            let ds = coverage_deficit_stats(&cov, &snap.mask512, &material_interior, size, m, coarse_size);
            println!(
                "    {name:<14} interior: mean_deficit={:.5} n={} | frontier: mean_deficit={:.5} n={}",
                ds.0, ds.1, ds.2, ds.3
            );
        }

        // Round 5, step 1: R7 sharpening sweep. `k` only ever multiplies the raw emptiness signal
        // before its own clamp (see `r7_emptiness`), so frontier behaviour (where raw is already
        // ~1 well before any clamp) is expected to stay put across k -- printed per-k so that
        // expectation is checked, not assumed.
        println!("    -- round 5: R7 sharpening sweep (emptiness = clamp(k*(1-h_min/h0),0,1)) --");
        for &k in &[1.0f32, 2.0, 3.0, 4.0] {
            let models = precompute_r7_models(&coarse, m, k);
            let (h, cov) = reconstruct_from_r6_models(&coarse, &models, m);
            let combined: Vec<f32> = (0..h.len()).map(|i| if cov[i] >= 0.5 { h[i] } else { -1.0 }).collect();
            let mass_err = max_mass_error(&coarse, &h, m);
            let cm = coverage_metrics(&snap.h, &combined, &snap.mask512, size, THRESH);
            let fn_split = fn_interior_frontier_split(&snap.h, &combined, THRESH, &snap.mask512, size, &material_interior, m, coarse_size);
            let deficit = coverage_deficit_stats(&cov, &snap.mask512, &material_interior, size, m, coarse_size);
            println!(
                "    k={k:<4} max_mass_err={:>10.6} iou={:.4} fp_px={:>5} fn_px={:>5} (interior={} frontier={}) interior_deficit={:.5} frontier_deficit={:.5}",
                mass_err, cm.iou, cm.fp, cm.fn_, fn_split.interior, fn_split.frontier, deficit.0, deficit.2
            );
            if !free_fall_rows.is_empty() {
                let agg = per_stream_width(&snap.h, &h, &snap.mask512, size, &free_fall_rows, THRESH);
                println!(
                    "        per-stream width occ_ratio_mean={:.3} mass_ratio_mean={:.3} missing={} extra={}",
                    agg.mean_occ_ratio, agg.mean_mass_ratio, agg.n_missing, agg.n_extra
                );
            }
            let rule_boundary = boundary_mask(&combined, &snap.mask512, size, THRESH);
            let rule_dt = chamfer_dt(&rule_boundary, size);
            for (label, region) in [("stream_sides", region_stream), ("pool_wall_edge", region_pool), ("slope_front", region_slope)] {
                let cf = front_fidelity(&orig_boundary, &orig_dt, &rule_boundary, &rule_dt, size, region);
                println!("        front_fidelity[{label}] fwd mean={:.3} max={:.3}", cf.fwd_mean, cf.fwd_max);
            }
        }

        // Round 5, "also, briefly": two candidate SECOND mechanisms for a regular grid-pattern
        // artifact, independent of R7/k entirely (neither check below depends on k at all).
        println!("    -- round 5: second-mechanism checks (independent of R7's k) --");
        let wstats = w_instability_stats(&coarse, &material_interior);
        println!(
            "    w-instability: mean|dw| between adjacent material-interior cells={:.4} (n_pairs={}) frac_extreme(w<0.1 or w>0.9)={:.3} (n_interior_cells={})",
            wstats.mean_abs_dw, wstats.n_pairs, wstats.frac_extreme, wstats.n_interior
        );
        let orig_bj = boundary_jump_stats(&snap.h, &snap.mask512, &material_interior, size, m, coarse_size);
        let ratio = |bj: &BoundaryJump| if bj.within_mean > 1e-12 { bj.boundary_mean / bj.within_mean } else { f64::NAN };
        println!(
            "    boundary-jump ORIGINAL  : boundary_mean={:.6} (n={}) within_mean={:.6} (n={}) ratio={:.3}",
            orig_bj.boundary_mean, orig_bj.n_boundary, orig_bj.within_mean, orig_bj.n_within, ratio(&orig_bj)
        );
        for (name, recon) in [
            ("R1".to_string(), reconstruct(&coarse, Rule::R1, m)),
            ("R6".to_string(), reconstruct_r6(&coarse, m).0),
            ("R7(k=1)".to_string(), reconstruct_r7(&coarse, m, 1.0).0),
        ] {
            let bj = boundary_jump_stats(&recon, &snap.mask512, &material_interior, size, m, coarse_size);
            println!(
                "    boundary-jump {name:<8}: boundary_mean={:.6} (n={}) within_mean={:.6} (n={}) ratio={:.3}",
                bj.boundary_mean, bj.n_boundary, bj.within_mean, bj.n_within, ratio(&bj)
            );
        }
    }

    // R1 threshold sensitivity, m=2 only (representative).
    {
        let m = 2usize;
        let coarse_size = size / m;
        let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
        let (coarse, _) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
        let recon = reconstruct(&coarse, Rule::R1, m);
        println!("\n-- R1 coverage at alternative thresholds (m=2) --");
        for &t in &[0.001f32, 0.003, 0.01, 0.02, 0.05] {
            let cov = coverage_metrics(&snap.h, &recon, &snap.mask512, size, t);
            println!("  thresh={t:<6} fp_px={:<8} fn_px={:<8} iou={:.4}", cov.fp, cov.fn_, cov.iou);
        }
    }
}

/// Round-2 fix D: 128px base crops were too small to judge staircasing by eye. Every crop is
/// still sampled at this same 128px base (identical regions to round 1 -- `stream_region`/
/// `pool_wall_region`/`slope_region` are shared with `run_snapshot`'s numeric probes above, so the
/// picture and the number are always of the same patch), then magnified 4x nearest-neighbour to
/// 512px per tile before saving, per the round-2 brief.
const MAGNIFY: u32 = 4;
/// Contact sheets show the shipped rule (R1), R3 (the best-measuring limited plane -- R2 is its
/// close twin and stays in the numeric tables only), R4 (PLIC), R5 (anti-aliased PLIC), R6
/// (round 3's centred-strip extension of R5), and R7 (round 4's interior-saturating extension of
/// R6, the direct fix for the "quilting" verdict on R6).
const PICTURE_RULES: [Rule; 6] = [Rule::R1, Rule::R3, Rule::R4, Rule::R5, Rule::R6, Rule::R7];

/// Round-3 fix #1: the page displays the EMA field (snapshot c), not the raw per-tick field
/// (snapshot a) -- pictures must judge what's actually shown. Every figure that was built from (a)
/// alone in round 2 is now built from BOTH, at the IDENTICAL crop region (computed once from (a)
/// and reused for (c), so the two are a fair side-by-side rather than each finding its own best
/// spot), so the effect of temporal smoothing on speckle/jitter can be read directly off the two
/// contact sheets for the same feature. `sand_slope` has no water/EMA counterpart (dry sand,
/// scenario b) so it stays single-snapshot, renamed only for the "every figure states its
/// snapshot" requirement.
fn make_all_pictures(snap_a: &Snapshot, snap_b: &Snapshot, snap_c: &Snapshot, out_dir: &std::path::Path) {
    let size = snap_a.sim.heightmap.width;

    let region_stream = stream_region(&snap_a.mask512, size, 128);
    let region_pool = pool_wall_region(&snap_a.mask512, &snap_a.h, size, 128);
    let region_slope = slope_region(&snap_b.mask512, &snap_b.h, size, 128);

    let figures: [(&str, &Snapshot, (usize, usize, usize, usize), f32); 5] = [
        ("stream_neck_a_raw", snap_a, region_stream, 0.55),
        ("stream_neck_c_ema", snap_c, region_stream, 0.55),
        ("pool_wall_edge_a_raw", snap_a, region_pool, 0.55),
        ("pool_wall_edge_c_ema", snap_c, region_pool, 0.55),
        ("sand_slope_b_dry", snap_b, region_slope, 0.5),
    ];

    let mut readme = String::new();
    readme.push_str("# Picture crops -- artifacts/design/upscale-2026-09-14/\n\n");
    readme.push_str(&format!(
        "Every crop is a 128x128 sample of the 512 grid, magnified {MAGNIFY}x nearest-neighbour to \
        512x512 (no new information -- purely so staircasing is legible). Grayscale = height; \
        orange = the coverage boundary (`h>=0.003` for R0-R4; for R5/R6/R7, `coverage>=0.5 AND \
        coverage*height>=0.003` -- round 3's fix for the film-case fleck defect, see the writeup \
        §4.3). Figure name suffix states the snapshot: `_a_raw_` = raw per-tick snapshot (a), \
        `_c_ema_` = the alpha=0.4 EMA over the last 15 ticks (snapshot c, what the deployed page \
        actually displays), `_b_dry_` = the DrySand snapshot (b, no EMA counterpart). \
        `stream_neck` and `pool_wall_edge` are shown at BOTH (a) and (c), at the IDENTICAL crop \
        region, specifically so temporal smoothing's effect on speckle/jitter can be read directly \
        off the two sheets for the same feature.\n\n"
    ));

    for (fig_name, snap, (x0, y0, w, hh), vmax) in figures {
        let orig_img = magnify_nearest(&crop_image(&snap.h, &snap.h, THRESH, &snap.mask512, size, x0, y0, w, hh, vmax), MAGNIFY);
        orig_img.save(out_dir.join(format!("{fig_name}_original.png"))).expect("write png");

        readme.push_str(&format!("## {fig_name}\ncrop = (x0={x0}, y0={y0}, w={w}, h={hh}), vmax={vmax}\n\n"));
        readme.push_str(&format!(
            "`{fig_name}_contact_sheet.png`: 7 columns x 2 rows. Row 1 = m=2, row 2 = m=4. Columns \
            left to right:\n\n"
        ));
        readme.push_str("| col 1 | col 2 | col 3 | col 4 | col 5 | col 6 | col 7 |\n|---|---|---|---|---|---|---|\n");
        readme.push_str("| original | R1 bilinear (shipped) | R3 face-match | R4 PLIC | R5 PLIC+AA | R6 strip+AA | R7 strip+sat |\n\n");

        let mut sheet_tiles = vec![orig_img.clone()];
        for &m in &[2usize, 4usize] {
            if m == 4 {
                // Original repeats as column 1 of row 2 for direct side-by-side comparison.
                sheet_tiles.push(orig_img.clone());
            }
            let coarse_size = size / m;
            let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
            let (coarse, _) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
            for &rule in &PICTURE_RULES {
                let (recon, cov_arr, cov_thresh) = reconstruct_pair(&coarse, rule, m);
                let base = crop_image(&recon, &cov_arr, cov_thresh, &snap.mask512, size, x0, y0, w, hh, vmax);
                let img = magnify_nearest(&base, MAGNIFY);
                let fname = format!("{fig_name}_{}_m{m}.png", rule.name());
                img.save(out_dir.join(&fname)).expect("write png");
                sheet_tiles.push(img);
            }
        }
        // R0/R2 individual crops too (not in the contact sheet, but on disk for reference).
        for &m in &[2usize, 4usize] {
            let coarse_size = size / m;
            let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
            let (coarse, _) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
            for &rule in &[Rule::R0, Rule::R2] {
                let (recon, cov_arr, cov_thresh) = reconstruct_pair(&coarse, rule, m);
                let base = crop_image(&recon, &cov_arr, cov_thresh, &snap.mask512, size, x0, y0, w, hh, vmax);
                let img = magnify_nearest(&base, MAGNIFY);
                img.save(out_dir.join(format!("{fig_name}_{}_m{m}.png", rule.name()))).expect("write png");
            }
        }

        let sheet = make_contact_sheet(&sheet_tiles, 7);
        sheet.save(out_dir.join(format!("{fig_name}_contact_sheet.png"))).expect("write contact sheet");
        println!("wrote {fig_name}: crop=({x0},{y0},{w}x{hh}) vmax={vmax} magnify={MAGNIFY}x");
    }

    std::fs::write(out_dir.join("README.md"), readme).expect("write README");
}

fn selftest_geometry() {
    // area_frac_exact(1,0,0) on the whole unit cell should be exactly 0.5 (a vertical line through
    // the centre, normal pointing +x, covers the right half).
    println!("selftest area_frac_exact(1,0,0) = {} (want 0.5)", area_frac_exact(1.0, 0.0, 0.0));
    println!("selftest area_frac_exact(0,1,0) = {} (want 0.5)", area_frac_exact(0.0, 1.0, 0.0));
    println!("selftest area_frac_exact(1,0,0.5) = {} (want 0.0, line at the right edge)", area_frac_exact(1.0, 0.0, 0.5));
    println!("selftest area_frac_exact(1,0,-0.5) = {} (want 1.0, line at the left edge)", area_frac_exact(1.0, 0.0, -0.5));
    // box_area_frac on a tiny box far from the interface should be exactly 0 or 1, never a stray
    // partial value.
    println!("selftest box_area_frac(1,0,0.5, cx=-0.4,cy=0,half=0.02) = {} (want 0.0, box entirely left of an interface at x=0.5)", box_area_frac(1.0, 0.0, 0.5, -0.4, 0.0, 0.02));
    println!("selftest box_area_frac(1,0,-0.5, cx=-0.4,cy=0,half=0.02) = {} (want 1.0, box entirely right of an interface at x=-0.5)", box_area_frac(1.0, 0.0, -0.5, -0.4, 0.0, 0.02));
    // A cell whose gradient is near-zero (flat) but h0 slightly below the neighbour max should
    // give f close to but below 1 -- solve_plic_threshold should then place t so nearly the whole
    // cell is covered, not scattered slivers.
    let f = 0.98f32;
    let t = solve_plic_threshold(0.0, -1.0, f);
    println!("selftest solve_plic_threshold(nx=0,ny=-1,f=0.98) = t={t}, area_frac_exact at that t = {} (want ~0.98)", area_frac_exact(0.0, -1.0, t));

    // overlap_1d sanity: a strip covering the whole pixel, half, and none of it.
    println!("selftest overlap_1d(center=0,half=0.5, pixel=0,half=0.1) = {} (want 1.0, strip covers whole cell)", overlap_1d(0.0, 0.5, 0.0, 0.1));
    println!("selftest overlap_1d(center=0.25,half=0.1, pixel=0.25,half=0.1) = {} (want 1.0, exact overlap)", overlap_1d(0.25, 0.1, 0.25, 0.1));
    println!("selftest overlap_1d(center=-0.4,half=0.05, pixel=0.4,half=0.05) = {} (want 0.0, far apart)", overlap_1d(-0.4, 0.05, 0.4, 0.05));

    // R5 vs R6 in the single-axis-dominant case (round 3's "must degrade continuously into R5"
    // requirement): a 3x3 field, flat in y, with the centre's right neighbour full and left
    // neighbour empty. R6's gradient-free per-axis construction should closely match R5's
    // gradient-derived interface line here, since there IS a well-defined single direction.
    {
        let size = 3usize;
        let (h_ref, h0) = (1.0f32, 0.3f32);
        let mut h = vec![h0; size * size];
        let idx = |x: usize, y: usize| y * size + x;
        h[idx(0, 1)] = 0.0;
        h[idx(2, 1)] = h_ref;
        let mask = vec![MASK_INSIDE; size * size];
        let coarse = CoarseField { size, h, mask };
        let m = 4usize;
        let r5 = precompute_r5_models(&coarse, m);
        let r6 = precompute_r6_models(&coarse, m);
        let center = idx(1, 1);
        let mut max_diff = 0.0f32;
        for j in 0..m {
            for i in 0..m {
                let dx = (i as f32 + 0.5) / m as f32 - 0.5;
                let dy = (j as f32 + 0.5) / m as f32 - 0.5;
                let (h5, _) = eval_r5(&r5[center], dx, dy, m);
                let (h6, _) = eval_r6(&r6[center], dx, dy, m);
                max_diff = max_diff.max((h5 - h6).abs());
            }
        }
        println!("selftest R5 vs R6 single-axis-dominant max |diff| over m*m samples = {max_diff:.4} (want small, R6 degrades toward R5 here)");
    }

    // Round-3 fix #2 demonstration: a nearly-empty cell whose neighbours are ALSO nearly empty
    // (the film case) gets f close to 1 (fully "covered") even though its actual height is tiny.
    // Round 2's `coverage>=0.5` alone would draw this as a visible fleck; round 3's
    // `coverage>=0.5 AND coverage*height>=0.003` must not.
    {
        let size = 3usize;
        let h0 = 0.0005f32; // above 0 but far below the 0.003 shader opacity threshold
        let h = vec![h0; size * size]; // every neighbour equally tiny -> f ~= 1
        let mask = vec![MASK_INSIDE; size * size];
        let coarse = CoarseField { size, h, mask };
        let m = 2usize;
        let r5 = precompute_r5_models(&coarse, m);
        let center = 1 * size + 1;
        let (hv, cv) = eval_r5(&r5[center], 0.0, 0.0, m);
        let would_be_covered_round2 = cv >= 0.5;
        let is_covered_round3 = cv >= 0.5 && hv >= THRESH;
        println!(
            "selftest film-case fleck: h0={h0}, R5 coverage={cv:.4} height={hv:.6} -- round2 'covered'={would_be_covered_round2}, round3 'covered'={is_covered_round3} (want true, false)"
        );
    }
}

fn main() {
    if std::env::var("SELFTEST").is_ok() {
        selftest_geometry();
        return;
    }
    let out_dir = output_dir();
    println!("output dir: {}", out_dir.display());

    println!("building snapshot (a): MultiNeckHourglass water, mid-drain, 600 ticks...");
    let snap_a = snapshot_a();
    println!("building snapshot (b): Hourglass DrySand, draining, 900 ticks...");
    let snap_b = snapshot_b();
    println!("building snapshot (c): same as (a), EMA alpha=0.4 over last 15 ticks...");
    let snap_c = snapshot_c_ema();

    run_snapshot(&snap_a);
    run_snapshot(&snap_b);
    run_snapshot(&snap_c);

    println!("\n================ writing picture crops ================");
    make_all_pictures(&snap_a, &snap_b, &snap_c, &out_dir);

    println!("\ndone.");
}
