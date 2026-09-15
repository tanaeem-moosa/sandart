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
//! of the fine mask), then re-upscales with five candidate rules and compares each against the
//! original 512 field. Comparing separate 256/512 simulations would conflate reconstruction error
//! with the fact that different resolutions evolve at different rates -- see CLAUDE.md's method
//! note. All five rules take ONLY (height field, mask) as input -- no material/wetness branch --
//! satisfying "the same rule for all materials" by construction, not by convention.
//!
//! Run: `cargo run -p sandart-sim --release --example diag_upscale_reconstruction`
//!
//! Writes PNG crops to `artifacts/design/upscale-2026-09-14/` (resolved relative to this crate's
//! `CARGO_MANIFEST_DIR`, so it works regardless of the caller's cwd) and prints all metric tables
//! to stdout.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, MASK_OUTSIDE};

const THRESH: f32 = 0.003;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule { R0, R1, R2, R3, R4 }

impl Rule {
    fn name(&self) -> &'static str {
        match self {
            Rule::R0 => "R0_nearest",
            Rule::R1 => "R1_bilinear",
            Rule::R2 => "R2_limited_plane",
            Rule::R3 => "R3_face_match",
            Rule::R4 => "R4_plic",
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
    }
    out
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

struct StreamWidth { occupied_px: usize, mass_equiv: f64 }

fn stream_width_at_row(h: &[f32], mask: &[u8], size: usize, row: usize) -> StreamWidth {
    let mut occupied = 0usize;
    let mut mass = 0.0f64;
    let mut peak = 0.0f32;
    for x in 0..size {
        let i = row * size + x;
        if mask[i] == MASK_OUTSIDE {
            continue;
        }
        let v = h[i];
        if v >= THRESH {
            occupied += 1;
        }
        mass += v as f64;
        peak = peak.max(v);
    }
    StreamWidth { occupied_px: occupied, mass_equiv: if peak > 0.0 { mass / peak as f64 } else { 0.0 } }
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
// PNG output
// ---------------------------------------------------------------------------------------------

fn crop_image(h: &[f32], mask: &[u8], size: usize, x0: usize, y0: usize, w: usize, hh: usize, vmax: f32) -> image::RgbImage {
    let mut img = image::RgbImage::new(w as u32, hh as u32);
    for by in 0..hh {
        for bx in 0..w {
            let (x, y) = (x0 + bx, y0 + by);
            let idx = y * size + x;
            let outside = mask[idx] == MASK_OUTSIDE;
            let v = (h[idx] / vmax).clamp(0.0, 1.0);
            let g = (v * 235.0) as u8 + if outside { 0 } else { 20 };
            let mut px = if outside { [30u8, 26u8, 22u8] } else { [g, g, g] };
            // Coverage outline: colour a pixel if it's covered (h>=THRESH) and at least one of its
            // 4-neighbours is not, i.e. the h>=THRESH boundary.
            if !outside {
                let covered = h[idx] >= THRESH;
                let mut edge = false;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx as usize >= size || ny as usize >= size {
                        continue;
                    }
                    let nidx = ny as usize * size + nx as usize;
                    let n_covered = mask[nidx] != MASK_OUTSIDE && h[nidx] >= THRESH;
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

fn make_contact_sheet(tiles: &[image::RgbImage]) -> image::RgbImage {
    let cols = tiles.len().min(6);
    let rows = (tiles.len() + cols - 1) / cols;
    let (tw, th) = tiles.first().map(|t| (t.width(), t.height())).unwrap_or((1, 1));
    let pad = 4u32;
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

fn run_snapshot(snap: &Snapshot) {
    let size = snap.sim.heightmap.width;
    println!("\n================ snapshot: {} (S=512) ================", snap.name);

    for &m in &[2usize, 4usize] {
        let coarse_size = size / m;
        let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
        let (coarse, dreport) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
        let coarse_interior = coarse_interior_mask(&coarse);

        let loss_pct = if dreport.total_mass > 0.0 { 100.0 * dreport.mass_lost / dreport.total_mass } else { 0.0 };
        println!(
            "\n-- m={m} (coarse S={coarse_size}) -- downscale mass lost to blocks outside the coarse mask: {:.6} / {:.6} total ({:.4}%)",
            dreport.mass_lost, dreport.total_mass, loss_pct
        );

        // Neck rows (scenario a/c only meaningful, but harmless elsewhere) and stream widths.
        let neck_rows_fine = find_neck_rows(&snap.mask512, size);
        println!("detected neck rows (fine, S=512): {neck_rows_fine:?}");
        if std::env::var("DUMP_ROW_COUNTS").is_ok() {
            let counts: Vec<usize> = (0..size).map(|y| (0..size).filter(|&x| snap.mask512[y * size + x] != MASK_OUTSIDE).count()).collect();
            for (y, c) in counts.iter().enumerate() {
                if y % 4 == 0 { println!("row {y}: {c}"); }
            }
        }
        let probe_rows: Vec<usize> = neck_rows_fine.iter().filter_map(|&r| if r + 3 < size { Some(r + 2) } else { None }).collect();

        // Pool region (for smoothness) and slope region (only used for picture crops later).
        let pool_box = 24usize;
        let (pool_x, pool_y) = find_pool_region(&snap.mask512, &snap.h, size, pool_box);
        let orig_smooth = second_diff_rms(&snap.h, size, pool_x, pool_y, pool_box);

        println!(
            "original pool-interior 2nd-diff RMS at ({pool_x},{pool_y}) box={pool_box}: {:.9} (near-zero is expected for a settled pool/pile plateau -- this is the TRUE baseline, not noise)",
            orig_smooth
        );
        println!(
            "{:<18} {:>12} {:>10} {:>10} {:>10} {:>8} {:>8} {:>8} {:>12}",
            "rule", "max_mass_err", "rms_all", "rms_int", "rms_front", "fp_px", "fn_px", "iou", "smooth_abs"
        );

        for &rule in &[Rule::R0, Rule::R1, Rule::R2, Rule::R3, Rule::R4] {
            let recon = reconstruct(&coarse, rule, m);
            let mass_err = max_mass_error(&coarse, &recon, m);
            let rms = height_rms(&snap.h, &recon, &snap.mask512, size, &coarse_interior, m, coarse_size);
            let cov = coverage_metrics(&snap.h, &recon, &snap.mask512, size, THRESH);
            let recon_smooth = second_diff_rms(&recon, size, pool_x, pool_y, pool_box);

            println!(
                "{:<18} {:>12.6} {:>10.6} {:>10.6} {:>10.6} {:>8} {:>8} {:>8.4} {:>12.9}",
                rule.name(), mass_err, rms.all, rms.interior, rms.frontier, cov.fp, cov.fn_, cov.iou, recon_smooth
            );

            if !probe_rows.is_empty() {
                for &row in &probe_rows {
                    let orig_w = stream_width_at_row(&snap.h, &snap.mask512, size, row);
                    let rule_w = stream_width_at_row(&recon, &snap.mask512, size, row);
                    println!(
                        "    stream row {row}: {} occupied_px={} (orig {}), mass_equiv_width={:.3} (orig {:.3}, ratio {:.3})",
                        rule.name(), rule_w.occupied_px, orig_w.occupied_px, rule_w.mass_equiv, orig_w.mass_equiv,
                        if orig_w.mass_equiv > 1e-9 { rule_w.mass_equiv / orig_w.mass_equiv } else { f64::NAN }
                    );
                }
            }
        }
        println!("(rms/mass units are heightmap units 0..1; pixel counts are over the full 512x512 fine-inside domain)");
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

fn make_all_pictures(snap_a: &Snapshot, snap_b: &Snapshot, out_dir: &std::path::Path) {
    let size = snap_a.sim.heightmap.width;

    // Crop 1: water stream + neck. Crop 2: pool edge against a wall. Both from scenario (a).
    let neck_rows = find_neck_rows(&snap_a.mask512, size);
    let neck_row = *neck_rows.first().unwrap_or(&(size / 3));
    let stream_crop = (size / 2 - 64, neck_row.saturating_sub(10), 128usize, 128usize);

    let pool_box = 48usize;
    let (pool_x, pool_y) = find_pool_region(&snap_a.mask512, &snap_a.h, size, pool_box);
    // Slide the crop toward the nearest wall boundary within a small search window so the pool
    // edge itself (not just the interior) is inside frame.
    let mut wall_x = pool_x;
    for x in pool_x.saturating_sub(80)..(pool_x + pool_box + 80).min(size) {
        if snap_a.mask512[pool_y * size + x] == MASK_OUTSIDE {
            wall_x = x;
            break;
        }
    }
    let pool_crop_x0 = wall_x.saturating_sub(64).min(size.saturating_sub(128));
    let pool_crop = (pool_crop_x0, pool_y.saturating_sub(32), 128usize, 128usize);

    // Crop 3: sand slope front, from scenario (b).
    let slope_box = 128usize;
    let (slope_x, slope_y) = find_slope_region(&snap_b.mask512, &snap_b.h, size, slope_box);
    let slope_crop = (slope_x, slope_y, slope_box, slope_box);

    let figures: [(&str, &Snapshot, (usize, usize, usize, usize), f32); 3] = [
        ("stream_neck", snap_a, stream_crop, 0.55),
        ("pool_wall_edge", snap_a, pool_crop, 0.55),
        ("sand_slope", snap_b, slope_crop, 0.5),
    ];

    for (fig_name, snap, (x0, y0, w, hh), vmax) in figures {
        let orig_img = crop_image(&snap.h, &snap.mask512, size, x0, y0, w, hh, vmax);
        orig_img.save(out_dir.join(format!("{fig_name}_original.png"))).expect("write png");
        let mut sheet_tiles = vec![orig_img];

        for &m in &[2usize, 4usize] {
            let coarse_size = size / m;
            let coarse_mask = snap.sim.rasterize_shape_mask(coarse_size);
            let (coarse, _) = downscale(&snap.h, &snap.mask512, size, &coarse_mask, coarse_size, m);
            for &rule in &[Rule::R0, Rule::R1, Rule::R2, Rule::R3, Rule::R4] {
                let recon = reconstruct(&coarse, rule, m);
                let img = crop_image(&recon, &snap.mask512, size, x0, y0, w, hh, vmax);
                let fname = format!("{fig_name}_{}_m{m}.png", rule.name());
                img.save(out_dir.join(&fname)).expect("write png");
                sheet_tiles.push(img);
            }
        }
        let sheet = make_contact_sheet(&sheet_tiles);
        sheet.save(out_dir.join(format!("{fig_name}_contact_sheet.png"))).expect("write contact sheet");
        println!("wrote {fig_name}: crop=({x0},{y0},{w}x{hh}) vmax={vmax}");
    }
}

fn main() {
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
    make_all_pictures(&snap_a, &snap_b, &out_dir);

    println!("\ndone.");
}
