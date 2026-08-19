//! Coarse pressure geometry — HIERARCHICAL-PRESSURE.md §4, build step 1.
//!
//! **Nothing reads this module's output yet.** It exists standalone: a data structure derived
//! once from `shape_mask` (and, for `capacity`, from `cell_props` wetness), rebuilt only where
//! `DrawingSimulation::generate_shape_mask` is rebuilt, tested against a flood fill of the fine
//! mask. No coupling into `settle_tick`, no change to simulation behaviour. See the design doc's
//! §9 build order — this is step 1 ("coarse geometry only").
//!
//! ## Relationship to `block_size` (forward-compatibility note, not implemented here)
//!
//! The design's §2 ("The LOD block and the pressure cell are the same object") anticipates
//! `block_size` changing from `grid_size/32` to `grid_size/64` in a *separate, later* change, at
//! which point the LOD block and this module's coarse tile become the same footprint. **That later
//! change has now happened** (see `DrawingSimulation::new_with_size`'s `block_size` doc comment
//! and `artifacts/design/BLOCK-RESIZE.md`): `block_size` is `grid_size/64` (floored at 1, not 2 —
//! a different floor decision from this module's `available` flag below, for reasons given at the
//! `block_size` site). This module still has NO reference to `block_size` — `COARSE_GRID` below
//! remains an independent constant, and the two subsystems' `t`/`block_size` arithmetic is
//! duplicated rather than shared. They are numerically identical at every resolution where both
//! are defined (`t` here and `block_size` there both equal `grid_size/64` for grid_size >= 128),
//! but diverge exactly at grid 64: this module reports `available = false` (disabled) while the
//! LOD scheduler still produces a working `block_size = 1`, since the scheduler has no
//! double-counting constraint forcing it to disable. The de-duplication this comment originally
//! deferred is still deferred — nothing here reads `block_size`, and nothing in the LOD scheduler
//! reads `COARSE_GRID` — until the coupling in §5 of the design is actually built.

use crate::physics::cell_capacity_for;
use crate::{MASK_OUTSIDE, PROP_WETNESS};

/// Fixed coarse-grid resolution the hierarchical-pressure design settles on (§2): 64x64 coarse
/// cells at every fine grid size, so the tile edge length `t = grid_size / COARSE_GRID` shrinks
/// with resolution (8 at 512, 4 at 256, 2 at 128) rather than the coarse grid itself changing
/// shape. This is what makes `R` (representable depth, §1) resolution-invariant — the entire
/// point of the fixed-grid decision over a fixed-tile-size one.
pub const COARSE_GRID: usize = 64;

/// Coarse-tile geometry derived once from `shape_mask`, plus a `capacity` term derived from
/// `cell_props` wetness that is expected to go stale independently (see `refresh_capacity`).
///
/// All per-cell/per-edge vectors are always sized `coarse_n * coarse_n` (cells) or
/// `(coarse_n-1) * coarse_n` / `coarse_n * (coarse_n-1)` (edges) and zero/false-filled even when
/// `available` is false, so a caller that forgets to check `available` reads all-zero data
/// instead of panicking on an empty or mis-sized buffer.
#[derive(Debug, Clone)]
pub struct CoarseGeometry {
    /// Fine grid size this geometry was built from. `DrawingSimulation`'s shape mask is always
    /// square (`new_with_size` takes one `grid_size` for both axes), so this is both width and
    /// height.
    pub grid_size: usize,
    /// Coarse grid edge length in coarse cells. `COARSE_GRID` (64) for anything built via
    /// `build`; a smaller value is used only by tests exercising the edge/open-span algorithm
    /// against a hand-written mask too small to hold a real 64x64 coarse grid (see
    /// `build_with_coarse_n`).
    pub coarse_n: usize,
    /// Fine cells per coarse-tile edge, `grid_size / coarse_n`. `0` when `available` is false.
    pub t: usize,
    /// `false` when `t < 2`, i.e. `grid_size <= coarse_n` (at `coarse_n = COARSE_GRID = 64`,
    /// that means grid 64 and below — grid 64 is a shipped resolution and is exactly the
    /// degenerate case §2 names: `t = 1` would make the coarse grid identical to the fine grid,
    /// so `capacity[C]` would be a cell's own overfill pressure added to its own potential, a
    /// literal 2x double-count.
    ///
    /// **Chosen resolution: disable, not floor.** §2 offers two options — "the coarse level must
    /// be disabled below some grid size, or `t` floored at 2 with the pressure grid allowed to be
    /// finer than 64." Flooring `t` at 2 for grid 64 would mean a 32x32 coarse grid there, which
    /// directly contradicts §2's own "Decision: the pressure grid is 64x64, fixed, at every
    /// render resolution" — the fixed-64 choice exists specifically so `R` is resolution-
    /// invariant, and a grid that changes shape below one resolution reopens exactly the question
    /// that decision closed. Disabling is also literally what §2's own consequences list
    /// suggests ("The coarse level must be disabled below some grid size"). Since nothing reads
    /// this data yet, there is no behavioural cost to deferring the grid-64 question to whichever
    /// build step first tries to couple at that resolution.
    pub available: bool,
    /// `open_cells[C]`: count of fine cells in tile `C` with `shape_mask != MASK_OUTSIDE`.
    /// Row-major, `coarse_n * coarse_n`, index `cy * coarse_n + cx`.
    pub open_cells: Vec<u32>,
    /// `capacity[C]`: sum of `cell_capacity_for(wetness)` over the same cells. THE ONE FIELD
    /// EXPECTED TO GO STALE ON ITS OWN — see `refresh_capacity`.
    pub capacity: Vec<f32>,
    /// `inside[C] = open_cells[C] > 0`.
    pub inside: Vec<bool>,
    /// `k[e]` for the x-direction edge between tile `(cx, cy)` and `(cx+1, cy)`. Row-major over
    /// `cy` in `0..coarse_n`, `cx` in `0..coarse_n-1`: index `cy * (coarse_n-1) + cx`. Use
    /// `k_x_at` rather than indexing directly.
    pub k_x: Vec<f32>,
    /// `k[e]` for the y-direction edge between tile `(cx, cy)` and `(cx, cy+1)`. Row-major over
    /// `cy` in `0..coarse_n-1`, `cx` in `0..coarse_n`: index `cy * coarse_n + cx`. Use `k_y_at`
    /// rather than indexing directly.
    pub k_y: Vec<f32>,
}

impl CoarseGeometry {
    /// Production entry point: builds the fixed 64x64 coarse grid (`COARSE_GRID`) over
    /// `shape_mask`/`cell_props` at `grid_size`. Called from `DrawingSimulation::
    /// generate_shape_mask` — the only place this should ever be constructed in production code,
    /// so it is rebuilt exactly where, and only where, `shape_mask` itself is rebuilt.
    ///
    /// Gracefully — not by panicking — falls back to `empty(grid_size)` (i.e. `available =
    /// false`) if `shape_mask`'s length does not match `grid_size * grid_size`. This can only
    /// happen if a future caller breaks the "always square" assumption; a mismatched buffer
    /// producing an inert/unavailable geometry is preferable to a panic inside a function called
    /// from deep in `DrawingSimulation`'s hot construction/reset paths.
    pub fn build(shape_mask: &[u8], cell_props: &[f32], grid_size: usize) -> CoarseGeometry {
        Self::build_with_coarse_n(shape_mask, cell_props, grid_size, COARSE_GRID)
    }

    /// A zeroed, `available = false` geometry at the production `COARSE_GRID` size — the
    /// placeholder used before the first real `build()` call (e.g. mid-construction, before
    /// `shape_mask` exists yet) and the fallback `build` uses on a mismatched buffer.
    pub fn empty(grid_size: usize) -> CoarseGeometry {
        Self::empty_n(grid_size, COARSE_GRID)
    }

    /// Generalised builder, parameterised on the coarse grid edge length instead of hard-coding
    /// `COARSE_GRID`. Production code should always go through `build` (which pins `coarse_n =
    /// COARSE_GRID = 64`, per §2's fixed-grid decision) — this exists so unit tests can exercise
    /// the open-span/edge-conveyance algorithm against small, hand-written masks (e.g. a 4x4 fine
    /// mask with `coarse_n = 2`) without needing a real 64-coarse-cell mask to hand-author, while
    /// the algorithm itself stays identical to what production runs.
    pub(crate) fn build_with_coarse_n(
        shape_mask: &[u8],
        cell_props: &[f32],
        grid_size: usize,
        coarse_n: usize,
    ) -> CoarseGeometry {
        if coarse_n == 0 || shape_mask.len() != grid_size * grid_size {
            return Self::empty_n(grid_size, coarse_n.max(1));
        }
        let t = if coarse_n == 0 { 0 } else { grid_size / coarse_n };
        let available = t >= 2 && t * coarse_n == grid_size;
        let mut geo = Self::empty_n(grid_size, coarse_n);
        geo.t = if available { t } else { 0 };
        geo.available = available;
        if !available {
            return geo;
        }
        geo.rebuild_open_cells(shape_mask);
        geo.rebuild_edges(shape_mask);
        geo.refresh_capacity(shape_mask, cell_props);
        geo
    }

    fn empty_n(grid_size: usize, coarse_n: usize) -> CoarseGeometry {
        CoarseGeometry {
            grid_size,
            coarse_n,
            t: 0,
            available: false,
            open_cells: vec![0u32; coarse_n * coarse_n],
            capacity: vec![0.0f32; coarse_n * coarse_n],
            inside: vec![false; coarse_n * coarse_n],
            k_x: vec![0.0f32; coarse_n.saturating_sub(1) * coarse_n],
            k_y: vec![0.0f32; coarse_n * coarse_n.saturating_sub(1)],
        }
    }

    #[inline]
    pub fn idx(&self, cx: usize, cy: usize) -> usize {
        cy * self.coarse_n + cx
    }

    /// `k[e]` for the edge between `(cx, cy)` and `(cx+1, cy)`. `cx` must be `< coarse_n - 1`.
    #[inline]
    pub fn k_x_at(&self, cx: usize, cy: usize) -> f32 {
        self.k_x[cy * (self.coarse_n - 1) + cx]
    }

    /// `k[e]` for the edge between `(cx, cy)` and `(cx, cy+1)`. `cy` must be `< coarse_n - 1`.
    #[inline]
    pub fn k_y_at(&self, cx: usize, cy: usize) -> f32 {
        self.k_y[cy * self.coarse_n + cx]
    }

    /// Recomputes `open_cells`/`inside` from `shape_mask`. Private: always paired with
    /// `rebuild_edges` in `build_with_coarse_n`, since a geometry with stale `open_cells` but
    /// fresh edges (or vice versa) is not a state anything should observe.
    fn rebuild_open_cells(&mut self, shape_mask: &[u8]) {
        self.open_cells.iter_mut().for_each(|v| *v = 0);
        let t = self.t;
        let n = self.coarse_n;
        for fy in 0..self.grid_size {
            let cy = (fy / t).min(n - 1);
            let row = fy * self.grid_size;
            for fx in 0..self.grid_size {
                if shape_mask[row + fx] != MASK_OUTSIDE {
                    let cx = (fx / t).min(n - 1);
                    self.open_cells[cy * n + cx] += 1;
                }
            }
        }
        for i in 0..self.open_cells.len() {
            self.inside[i] = self.open_cells[i] > 0;
        }
    }

    /// Recomputes `k_x`/`k_y` from `shape_mask`. Per §4: `open_span[e]` is the count of fine-cell
    /// pairs straddling the shared `t`-cell boundary that are orthogonally adjacent and both
    /// mask-inside; `k[e] = open_span[e] / t`. Only scans the single row/column of fine cells
    /// directly on each boundary (`t` pairs per coarse edge), not the tile interiors — `k[e]` is a
    /// property of the boundary, not of what either tile contains away from it.
    fn rebuild_edges(&mut self, shape_mask: &[u8]) {
        let t = self.t;
        let g = self.grid_size;
        let n = self.coarse_n;

        // x-direction edges: between tile (cx, cy) and (cx+1, cy), boundary is the column pair
        // (left tile's last column, right tile's first column).
        for cx in 0..n.saturating_sub(1) {
            let left_col = cx * t + (t - 1);
            let right_col = left_col + 1;
            for cy in 0..n {
                let mut count: u32 = 0;
                for sy in (cy * t)..((cy + 1) * t) {
                    let row = sy * g;
                    if shape_mask[row + left_col] != MASK_OUTSIDE
                        && shape_mask[row + right_col] != MASK_OUTSIDE
                    {
                        count += 1;
                    }
                }
                self.k_x[cy * (n - 1) + cx] = count as f32 / t as f32;
            }
        }

        // y-direction edges: between tile (cx, cy) and (cx, cy+1), boundary is the row pair
        // (top tile's last row, bottom tile's first row).
        for cy in 0..n.saturating_sub(1) {
            let top_row = (cy * t + (t - 1)) * g;
            let bottom_row = top_row + g;
            for cx in 0..n {
                let mut count: u32 = 0;
                for sx in (cx * t)..((cx + 1) * t) {
                    if shape_mask[top_row + sx] != MASK_OUTSIDE
                        && shape_mask[bottom_row + sx] != MASK_OUTSIDE
                    {
                        count += 1;
                    }
                }
                self.k_y[cy * n + cx] = count as f32 / t as f32;
            }
        }
    }

    /// Recomputes `capacity` ONLY — geometry (`open_cells`, `inside`, `k_x`, `k_y`) is untouched.
    /// This is §4's flagged exception: `cell_capacity_for` depends on wetness, which advection
    /// changes continuously, so `capacity` is the one field this module expects to go stale
    /// between geometry rebuilds.
    ///
    /// **Cadence this is designed for (not yet wired to anything — nothing calls this on a
    /// timer in this build step, since nothing reads `capacity` yet either):** the design's own
    /// suggested middle ground is "recompute it on the same cadence as the saturation deciles
    /// rather than per tick" (§4) — i.e. `SATURATION_DECILE_REFRESH_TICKS` (30 ticks,
    /// `lib.rs`). That cadence was chosen there for a legibility reason (a *display* legend
    /// should not visibly jitter frame to frame); the reason it is *also* appropriate here is
    /// different and specific to this field: wetness mixes by advection, which is itself a slow,
    /// diffusive process relative to one tick, so a 30-tick-stale `capacity` is a bounded, small
    /// error against a quantity that does not move quickly. A future build step that actually
    /// reads `capacity` should call this on that cadence (or justify a different one) rather than
    /// inventing a fresh number.
    pub fn refresh_capacity(&mut self, shape_mask: &[u8], cell_props: &[f32]) {
        if !self.available {
            return;
        }
        if shape_mask.len() != self.grid_size * self.grid_size
            || cell_props.len() != self.grid_size * self.grid_size * 4
        {
            return;
        }
        self.capacity.iter_mut().for_each(|v| *v = 0.0);
        let t = self.t;
        let n = self.coarse_n;
        for fy in 0..self.grid_size {
            let cy = (fy / t).min(n - 1);
            let row = fy * self.grid_size;
            for fx in 0..self.grid_size {
                let fi = row + fx;
                if shape_mask[fi] != MASK_OUTSIDE {
                    let cx = (fx / t).min(n - 1);
                    let wetness = cell_props[fi * 4 + PROP_WETNESS];
                    self.capacity[cy * n + cx] += cell_capacity_for(wetness);
                }
            }
        }
    }
}

/// Default anchoring rate lambda for pulling M towards A: 0.10 (§5 step 2).
pub const COARSE_DEFAULT_LAMBDA: f32 = 0.10;
/// Default relaxation sweeps per tick on the 64x64 coarse grid: 16 sweeps (§5 step 3).
pub const COARSE_DEFAULT_SWEEPS: usize = 16;

/// Unbounded potential function for coarse compression (§4.3 / Step 0 recommendation).
/// Below capacity (h <= cap): phi = h / cap in [0, 1].
/// Above capacity (h > cap): phi = h / cap + unit * (h - cap) / cap.
#[inline]
pub fn coarse_unbounded_phi(h: f32, cap: f32, unit: f32) -> f32 {
    if cap <= 0.0 {
        return 0.0;
    }
    let x = h / cap;
    if x <= 1.0 {
        x
    } else {
        x + unit * (x - 1.0)
    }
}

/// Solves `phi(h_a - d, cap_a) + g - phi(h_b + d, cap_b) = 0` for `d` under the unbounded law.
#[inline]
pub fn coarse_unbounded_transfer(h_a: f32, cap_a: f32, h_b: f32, cap_b: f32, g: f32, unit: f32) -> f32 {
    if cap_a <= 0.0 || cap_b <= 0.0 {
        return 0.0;
    }
    let lo = -h_b;
    let hi = h_a;
    if hi <= lo {
        return 0.0;
    }
    let stress = |d: f32| {
        coarse_unbounded_phi(h_a - d, cap_a, unit) + g - coarse_unbounded_phi(h_b + d, cap_b, unit)
    };
    if stress(hi) >= 0.0 {
        return hi;
    }
    if stress(lo) <= 0.0 {
        return lo;
    }
    let mut lo = lo;
    let mut hi = hi;
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if stress(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Coarse simulation state (`sandart_sim::coarse`, HIERARCHICAL-PRESSURE.md §5, build step 2).
///
/// Holds the restricted fine mass observation `A[C]`, persistent coarse mass state `M[C]`,
/// coarse hydraulic head `eta[C]`, hydrostatic pressure `P[C]`, and signed disagreement `Delta[C]`.
#[derive(Debug, Clone)]
pub struct CoarseState {
    pub coarse_n: usize,
    /// A[C]: aggregated fine mass in tile C.
    pub a_mass: Vec<f32>,
    /// M[C]: persistent coarse mass in tile C across ticks.
    pub m_mass: Vec<f32>,
    /// eta[C]: coarse hydraulic head (phi_coarse - cy * base_head_coarse).
    pub eta: Vec<f32>,
    /// P[C]: coarse hydrostatic pressure (phi_coarse - M[C]/capacity[C]).
    pub p_coarse: Vec<f32>,
    /// Delta[C]: signed coarse-fine disagreement M[C] - A[C].
    pub delta: Vec<f32>,
    /// Support[C]: aggregated REAL fine overfill mass in tile C -- `sum((h_i - cap_i).max(0))`
    /// over the tile's fine cells, using the SAME nominal per-cell capacity
    /// (`cell_capacity_for(wetness)`) the fine solver's own `o = (h - cap) / cap` uses. See §5
    /// step 5's deadband derivation on `update_head_and_disagreement` for what this gates and why.
    pub support_mass: Vec<f32>,
    /// Grounded[C]: TRANSITIVE support -- true if tile C itself has `support_mass[C] > 0`, OR
    /// rests directly on the floor/casing (no open tile below), OR sits on top of a grounded
    /// tile (mass-connected via `k_y[e] > 0`). Computed by `compute_grounded` from
    /// `support_mass` and the (static) coarse geometry, mirroring `diag_support.rs`'s own
    /// `transitively_supported` fine-cell definition one level up. See the deadband derivation
    /// on `update_head_and_disagreement` for why LOCAL `support_mass` alone is not enough to gate
    /// on: it would also zero the eta signal for the un-compressed MIDDLE of a resting column
    /// (only its base is ever pushed over capacity), breaking exactly the depth-pressure
    /// propagation the coarse level exists to speed up.
    pub grounded: Vec<bool>,
    /// Anchor strength lambda in (0, 1].
    pub lambda: f32,
    /// Sweeps per tick.
    pub sweeps_n: usize,
}

impl CoarseState {
    pub fn new(coarse_n: usize) -> Self {
        let size = coarse_n * coarse_n;
        Self {
            coarse_n,
            a_mass: vec![0.0; size],
            m_mass: vec![0.0; size],
            eta: vec![0.0; size],
            p_coarse: vec![0.0; size],
            delta: vec![0.0; size],
            support_mass: vec![0.0; size],
            grounded: vec![false; size],
            lambda: COARSE_DEFAULT_LAMBDA,
            sweeps_n: COARSE_DEFAULT_SWEEPS,
        }
    }

    /// Step 1 of §5: Restrict fine heights into A[C] for inside cells. Also aggregates
    /// `support_mass[C]` -- see that field's doc comment and the deadband derivation on
    /// `update_head_and_disagreement`.
    pub fn restrict(&mut self, heights: &[f32], shape_mask: &[u8], cell_props: &[f32], geo: &CoarseGeometry) {
        if !geo.available || heights.len() != geo.grid_size * geo.grid_size || shape_mask.len() != heights.len()
            || cell_props.len() != heights.len() * 4 {
            return;
        }
        self.a_mass.iter_mut().for_each(|v| *v = 0.0);
        self.support_mass.iter_mut().for_each(|v| *v = 0.0);
        let t = geo.t;
        let n = self.coarse_n;
        let g = geo.grid_size;
        for fy in 0..g {
            let cy = (fy / t).min(n - 1);
            let row = fy * g;
            for fx in 0..g {
                let fi = row + fx;
                if shape_mask[fi] != MASK_OUTSIDE {
                    let cx = (fx / t).min(n - 1);
                    let ci = cy * n + cx;
                    let h = heights[fi];
                    self.a_mass[ci] += h;
                    // I5's deadband derivation: a fine cell can only carry `h > cap` (be
                    // compressed) if something stops its descent -- "compression is support"
                    // (§3, structurally exact: 0 of 9,647 falling cells ever have o > 0). This is
                    // the EXACT same per-cell capacity the fine solver's own `o = (h-cap)/cap`
                    // uses, so summing its excess over the tile gives an exact (not fitted) "does
                    // this tile contain any genuinely supported material" signal.
                    let cap = cell_capacity_for(cell_props[fi * 4 + PROP_WETNESS]);
                    if h > cap {
                        self.support_mass[ci] += h - cap;
                    }
                }
            }
        }
    }

    /// Step 2 of §5: Anchor M toward A by lambda: M += lambda * (A - M).
    pub fn anchor(&mut self, geo: &CoarseGeometry) {
        if !geo.available {
            return;
        }
        let lambda = self.lambda.clamp(0.0, 1.0);
        for i in 0..self.m_mass.len() {
            if geo.inside[i] {
                self.m_mass[i] += lambda * (self.a_mass[i] - self.m_mass[i]);
            } else {
                self.m_mass[i] = 0.0;
            }
        }
    }

    /// Step 3 of §5: Relax M across the coarse graph for N sweeps.
    pub fn relax(&mut self, sweeps: usize, base_head: f32, geo: &CoarseGeometry, unit: f32) {
        if !geo.available {
            return;
        }
        let n = self.coarse_n;
        let base_head_coarse = base_head * geo.t as f32;

        let mut delta_v = vec![0.0f32; n * n];
        for _ in 0..sweeps {
            delta_v.iter_mut().for_each(|v| *v = 0.0);

            // Vertical edges (y-direction, gravity-aligned)
            for cy in 0..n.saturating_sub(1) {
                for cx in 0..n {
                    let a = cy * n + cx;
                    let b = (cy + 1) * n + cx;
                    let k = geo.k_y_at(cx, cy);
                    if k <= 0.0 || geo.capacity[a] <= 0.0 || geo.capacity[b] <= 0.0 {
                        continue;
                    }
                    let d = k * coarse_unbounded_transfer(
                        self.m_mass[a], geo.capacity[a],
                        self.m_mass[b], geo.capacity[b],
                        base_head_coarse, unit,
                    );
                    delta_v[a] -= d;
                    delta_v[b] += d;
                }
            }
            for i in 0..n * n {
                self.m_mass[i] += delta_v[i];
            }

            delta_v.iter_mut().for_each(|v| *v = 0.0);
            // Horizontal edges (x-direction, no gravity)
            for cy in 0..n {
                for cx in 0..n.saturating_sub(1) {
                    let a = cy * n + cx;
                    let b = cy * n + cx + 1;
                    let k = geo.k_x_at(cx, cy);
                    if k <= 0.0 || geo.capacity[a] <= 0.0 || geo.capacity[b] <= 0.0 {
                        continue;
                    }
                    let d = k * coarse_unbounded_transfer(
                        self.m_mass[a], geo.capacity[a],
                        self.m_mass[b], geo.capacity[b],
                        0.0, unit,
                    );
                    delta_v[a] -= d;
                    delta_v[b] += d;
                }
            }
            for i in 0..n * n {
                self.m_mass[i] += delta_v[i];
            }
        }
    }

    /// Computes `grounded[C]` from `support_mass`: `grounded[C] = support_mass[C] > 0`, i.e. the
    /// tile itself contains real local compression -- see §3's structural guarantee ("compression
    /// is support") on `support_mass`'s own doc comment. Must run after `restrict` (needs fresh
    /// `support_mass`) and before `update_head_and_disagreement` (which reads `grounded`).
    ///
    /// **Deliberately LOCAL ONLY -- every attempt to extend it past a tile's own boundary was
    /// tried and measured WORSE**, on `diag_support --grid 512 --ticks 400` (hourglass, percent of
    /// free-falling cells carrying nonzero pressure; the plain local-only rule below measures
    /// 8.4%, the number every alternative here is compared against):
    /// - Propagating through any geometrically open tile below (`k_y[e] > 0`, ignoring how full
    ///   it is): 73.5%. An open neck lets "grounded" run straight up through a falling stream to
    ///   whatever pool it feeds.
    /// - Requiring the tile below to be "packed" (`a_mass/capacity >= 1 -
    ///   SUPPORT_FRACTION_EPSILON`) before inheriting its grounding, unbounded chain: 63.5%.
    /// - The same packed-below rule bounded to a single hop above a directly-compressed tile:
    ///   63.9%.
    /// - Treating direct floor/casing contact (no open tile below) as automatically grounded
    ///   whenever the tile holds any mass, matching the fine model's own "resting on the casing"
    ///   rule for `support_fraction`: 21.0%. Slanted hourglass walls put many tiles that are still
    ///   mid-funnel, not resting on anything, in contact with a `MASK_OUTSIDE` tile below through
    ///   no structural fault of their own.
    ///
    /// The common failure mode in all four: the tile immediately above (or beside, in the floor
    /// case) a genuinely compressed tile is very often the SAME tile the falling stream is still
    /// landing in (I5's "mixed tile... is the case that decides it"), and any rule that grants it
    /// grounding via a NEIGHBOUR's state reproduces exactly the "upward excursion at impact"
    /// acceptance criterion 4 forbids. Local-only gating cannot leak past a tile boundary by
    /// construction, which is why it is what ships, despite giving up the multi-tile-column
    /// speedup a transitive rule would have bought (§1's motivation) and despite an 8.4% residual
    /// that is not itself zero -- see the doc comment on `update_head_and_disagreement` for that
    /// residual and why it is the mixed-tile case, not a bug in this rule, and treat this as the
    /// open problem the design itself flags it as (§10), not as resolved.
    pub fn compute_grounded(&mut self, geo: &CoarseGeometry) {
        if !geo.available {
            self.grounded.iter_mut().for_each(|v| *v = false);
            return;
        }
        for c in 0..self.grounded.len() {
            self.grounded[c] = geo.inside[c] && geo.capacity[c] > 0.0 && self.support_mass[c] > 0.0;
        }
    }

    /// Step 4 of §5: Read back eta, P, and Delta.
    ///
    /// **Deadband (I5, HIERARCHICAL-PRESSURE.md §6/§8): the coarse PRESSURE term (`phi`'s excess
    /// above `x = M/cap` once `x > 1`) is withheld unless the tile is `grounded`** (see
    /// `compute_grounded`: real local compression, `support_mass[C] > 0` -- deliberately NOT
    /// transitive past the tile's own boundary; that doc comment records why every transitive
    /// variant tried measured worse). The below-capacity fill-fraction term (`phi = x` for
    /// `x <= 1`) is NEVER gated -- that is the harmless "water table" signal U-tube levelling
    /// needs even between two arms neither of which is near capacity (§0.3).
    ///
    /// **The elevation term (`- cy * base_head_coarse`) is ALSO never gated, on either branch.**
    /// A first version of this deadband zeroed `eta` outright for an ungrounded tile (both fill
    /// and elevation together) and measured WORSE than not gating at all: 55.2% of free-falling
    /// cells carrying nonzero pressure on `diag_support --grid 512 --ticks 400` (hourglass), up
    /// from an un-gated baseline nowhere near that. Stripping the elevation term from only ONE
    /// side of a grounded/ungrounded tile pair manufactures a spurious `delta_eta` from elevation
    /// ALONE -- worse the deeper the pair sits (`cy` larger) -- which is exactly the sawtooth
    /// §0.2 already solved once by insisting `eta`, not raw `phi`, is what crosses a tile seam.
    /// Gating only the pressure EXCESS, as implemented, keeps `eta` elevation-consistent on both
    /// sides of every edge regardless of grounding and measures 8.4%.
    ///
    /// **Honest caveat: 8.4% is barely different from 8.3% measured with the deadband OFF
    /// entirely** (excess unconditionally added, `grounded` short-circuited to always-true) on
    /// the same `diag_support` run. The excess term this deadband withholds is a SMALL
    /// contributor to this particular leak; the dominant one is the never-gated linear term
    /// (`phi = x` below capacity), which by the elevation-consistency argument above cannot be
    /// gated without reproducing the far worse (55.2%) failure. So this deadband is real and
    /// correctly derived -- it removes the one component that CAN be safely withheld -- but it is
    /// not, by itself, what gets `diag_support` close to zero. Report this residual honestly
    /// rather than as a solved acceptance criterion 4.
    ///
    /// Derived, not fitted: `support_mass` is the EXACT aggregate of real fine overfill
    /// (`sum((h_i - cap_i).max(0))`, computed straight from fine heights in `restrict`, using the
    /// same per-cell capacity the fine solver's own `o = (h - cap) / cap` uses). No epsilon
    /// constant anywhere in the decision -- the measured margin (worst falling-dominated tile at
    /// 0.9809, §3) was already thinner than any fitted constant could safely cover.
    ///
    /// **Known residual, not fully closed: 8.4%, not 0%.** The mixed tile (part stream, part
    /// pool) is the case I5 names as deciding this, and it is not fully solved here: as soon as
    /// ANY fine cell in a tile is compressed, `grounded[C]` is true and the WHOLE tile's pressure
    /// excess -- including whatever falling material shares that same tile, e.g. the impact zone
    /// directly above a pool's surface -- unlocks. This is bounded to that one tile's footprint
    /// (§7's potential coupling applies per tile, not per fine cell, so finer-than-a-tile
    /// discrimination is not available without abandoning the tile-granular coupling itself) but
    /// not eliminated. Treat
    /// §10's "this is the single largest unaddressed risk" as still true for the mixed-tile case
    /// specifically, and re-measure `diag_support` after any further change here.
    pub fn update_head_and_disagreement(&mut self, base_head: f32, geo: &CoarseGeometry, unit: f32) {
        if !geo.available {
            self.eta.iter_mut().for_each(|v| *v = 0.0);
            self.p_coarse.iter_mut().for_each(|v| *v = 0.0);
            self.delta.iter_mut().for_each(|v| *v = 0.0);
            return;
        }
        let n = self.coarse_n;
        let base_head_coarse = base_head * geo.t as f32;

        for cy in 0..n {
            for cx in 0..n {
                let c = cy * n + cx;
                let cap = geo.capacity[c];
                let m = self.m_mass[c];
                let a = self.a_mass[c];
                self.delta[c] = m - a;
                if cap > 0.0 && geo.inside[c] {
                    let x = m / cap;
                    // The elevation term (`- cy * base_head_coarse`) is NEVER gated, even for an
                    // ungrounded tile: an earlier version zeroed `eta` outright when `!grounded`,
                    // which measured WORSE (55.2%, up from 8.4%) than gating the pressure term
                    // alone -- zeroing eta strips the elevation baseline too, so a grounded tile
                    // deep in the grid (large `cy`, large `-cy*base_head_coarse`) sitting next to
                    // an ungrounded one reads a huge SPURIOUS `delta_eta` from elevation ALONE,
                    // worsening exactly at depth. Only `phi` -- the fill/pressure part -- is
                    // gated, and even then only its excess-above-capacity component: below
                    // capacity (`x <= 1`) `phi = x` unconditionally, since that is the harmless
                    // "water table" term U-tube levelling needs (§0.3) even between two arms
                    // neither of which is anywhere near capacity. Above capacity, an ungrounded
                    // tile's `phi` is capped at its capacity ceiling (`1.0`, not `x`): the excess
                    // pressure is exactly what the deadband exists to withhold.
                    let phi = if x <= 1.0 {
                        x
                    } else if self.grounded[c] {
                        x + unit * (x - 1.0)
                    } else {
                        1.0
                    };
                    self.eta[c] = phi - cy as f32 * base_head_coarse;
                    self.p_coarse[c] = (phi - x).max(0.0);
                } else {
                    self.eta[c] = 0.0;
                    self.p_coarse[c] = 0.0;
                }
            }
        }
    }

    /// Full tick update (Step 1 -> Step 2 -> Step 3 -> Step 4).
    pub fn tick(&mut self, heights: &[f32], shape_mask: &[u8], cell_props: &[f32], base_head: f32, geo: &CoarseGeometry, unit: f32) {
        if !geo.available {
            return;
        }
        self.restrict(heights, shape_mask, cell_props, geo);
        self.compute_grounded(geo);
        self.anchor(geo);
        self.relax(self.sweeps_n, base_head, geo, unit);
        self.update_head_and_disagreement(base_head, geo, unit);
    }

    /// Reset coarse mass M to match current aggregated mass A (e.g. on flip or shape change).
    pub fn reset_to_aggregated(&mut self) {
        self.m_mass.copy_from_slice(&self.a_mass);
        self.delta.iter_mut().for_each(|v| *v = 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawingSimulation, SandboxShape};
    use std::collections::VecDeque;

    /// 4-connected flood fill over `mask` (`w x h`), `inside = mask[i] != MASK_OUTSIDE`.
    /// Returns a component id per cell, `-1` for outside cells. Independent of
    /// `CoarseGeometry`'s own algorithm — used as ground truth for the connectivity tests, not
    /// exercised by production code.
    fn flood_fill_components(mask: &[u8], w: usize, h: usize) -> Vec<i32> {
        let mut labels = vec![-1i32; w * h];
        let mut next_label = 0i32;
        let mut queue = VecDeque::new();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if mask[i] == MASK_OUTSIDE || labels[i] != -1 {
                    continue;
                }
                labels[i] = next_label;
                queue.push_back((x, y));
                while let Some((cx, cy)) = queue.pop_front() {
                    let neighbors = [
                        (cx.wrapping_sub(1), cy),
                        (cx + 1, cy),
                        (cx, cy.wrapping_sub(1)),
                        (cx, cy + 1),
                    ];
                    for (nx, ny) in neighbors {
                        if nx >= w || ny >= h {
                            continue;
                        }
                        let ni = ny * w + nx;
                        if mask[ni] != MASK_OUTSIDE && labels[ni] == -1 {
                            labels[ni] = next_label;
                            queue.push_back((nx, ny));
                        }
                    }
                }
                next_label += 1;
            }
        }
        labels
    }

    /// Ground truth for "are tiles (cx,cy) and (cx+1,cy) connected across their shared
    /// boundary": true iff some fine-cell pair directly straddling that boundary is both inside
    /// AND (by the flood fill) in the same component. Since two orthogonally-adjacent inside
    /// cells are trivially in the same component (they are graph-adjacent by construction), this
    /// reduces to "both inside" — but it is computed independently of `CoarseGeometry::
    /// rebuild_edges` (different loop, using the flood-fill labels rather than a raw mask
    /// re-read) so it is a real check on that function's arithmetic, not a tautology restated.
    fn ground_truth_k_x(labels: &[i32], w: usize, t: usize, cx: usize, cy: usize) -> bool {
        let left_col = cx * t + (t - 1);
        let right_col = left_col + 1;
        (cy * t..(cy + 1) * t).any(|sy| {
            let row = sy * w;
            let a = labels[row + left_col];
            let b = labels[row + right_col];
            a != -1 && b != -1 && a == b
        })
    }

    fn ground_truth_k_y(labels: &[i32], w: usize, t: usize, cx: usize, cy: usize) -> bool {
        let top_row = (cy * t + (t - 1)) * w;
        let bottom_row = top_row + w;
        (cx * t..(cx + 1) * t).any(|sx| {
            let a = labels[top_row + sx];
            let b = labels[bottom_row + sx];
            a != -1 && b != -1 && a == b
        })
    }

    fn all_shapes() -> [SandboxShape; 10] {
        [
            SandboxShape::Circle,
            SandboxShape::Square,
            SandboxShape::Oval,
            SandboxShape::Hourglass,
            SandboxShape::MultiStageHourglass,
            SandboxShape::GaltonBoard,
            SandboxShape::StaircaseCascade,
            SandboxShape::ProceduralFunnel,
            SandboxShape::MultiNeckHourglass,
            SandboxShape::UTubeFlowThrough,
        ]
    }

    /// Test 1 (task spec): the connectivity guarantee, §4. `k[e] > 0` iff a flood fill of the
    /// fine mask says the two tiles are orthogonally connected across that specific boundary.
    /// Asserted across every shipped `SandboxShape` at grids 128, 256, 512.
    #[test]
    fn connectivity_matches_fine_flood_fill_across_all_shapes_and_grids() {
        for &grid in &[128usize, 256, 512] {
            for &shape in &all_shapes() {
                let mut sim = DrawingSimulation::new_with_size(grid);
                sim.sandbox_shape = shape;
                sim.generate_shape_mask();
                assert!(
                    sim.coarse.available,
                    "grid {grid} shape {shape:?}: coarse geometry should be available"
                );
                let geo = &sim.coarse;
                let labels = flood_fill_components(&sim.shape_mask, grid, grid);
                let t = geo.t;
                let n = geo.coarse_n;

                for cy in 0..n {
                    for cx in 0..n.saturating_sub(1) {
                        let k = geo.k_x_at(cx, cy);
                        let truth = ground_truth_k_x(&labels, grid, t, cx, cy);
                        assert_eq!(
                            k > 0.0,
                            truth,
                            "grid {grid} shape {shape:?}: k_x mismatch at ({cx},{cy}), k={k}"
                        );
                    }
                }
                for cy in 0..n.saturating_sub(1) {
                    for cx in 0..n {
                        let k = geo.k_y_at(cx, cy);
                        let truth = ground_truth_k_y(&labels, grid, t, cx, cy);
                        assert_eq!(
                            k > 0.0,
                            truth,
                            "grid {grid} shape {shape:?}: k_y mismatch at ({cx},{cy}), k={k}"
                        );
                    }
                }
            }
        }
    }

    /// Test 2 (task spec): the diagonal-neck case, §4. A neck crossing a tile corner at 45
    /// degrees can give zero orthogonally-adjacent pairs on BOTH shared edges even though the
    /// fine grid "looks" connected through the corner touch. The design argues `k = 0` is
    /// CORRECT there (the fine solver only moves mass orthogonally, so a diagonal-only touch
    /// carries no fine flow either) and flags this as the most likely place for the connectivity
    /// guarantee to be wrong. Verified here, not assumed.
    ///
    /// Hand-built 4x4 fine mask, coarse_n = 2 (t = 2), so the four coarse tiles are the four 2x2
    /// quadrants. Only two fine cells are inside: (1,1) — the bottom-right corner of the
    /// top-left tile — and (2,2) — the top-left corner of the bottom-right tile. These are
    /// diagonal neighbors (offset (1,1)), never orthogonal neighbors, so under 4-connectivity
    /// they are not even in the same flood-fill component: the "neck" is a pure corner touch,
    /// the sharpest version of the case §4 warns about.
    #[test]
    fn diagonal_corner_touch_gives_zero_conveyance_on_both_shared_edges() {
        let grid = 4usize;
        let mut mask = vec![MASK_OUTSIDE; grid * grid];
        mask[1 * grid + 1] = crate::MASK_INSIDE; // (x=1, y=1): corner of tile (0,0)
        mask[2 * grid + 2] = crate::MASK_INSIDE; // (x=2, y=2): corner of tile (1,1)
        let cell_props = vec![0.0f32; grid * grid * 4];

        let geo = CoarseGeometry::build_with_coarse_n(&mask, &cell_props, grid, 2);
        assert!(geo.available, "t=2 should be available");
        assert_eq!(geo.t, 2);

        // Ground truth: are the two inside cells orthogonally connected at all? They are not —
        // confirm that first, so the k=0 result below is not vacuously "everything is 0".
        let labels = flood_fill_components(&mask, grid, grid);
        assert_ne!(
            labels[1 * grid + 1],
            labels[2 * grid + 2],
            "the two marked cells must NOT be in the same flood-fill component \
             (diagonal touch only) for this to test the corner case"
        );
        assert_eq!(labels[1 * grid + 1], 0);
        assert_eq!(labels[2 * grid + 2], 1);

        // The two edges that border the shared corner: x-edge between tile (0,0)-(1,0), and
        // y-edge between tile (0,0)-(0,1). Both must read k = 0.
        let k_x = geo.k_x_at(0, 0); // (0,0)-(1,0)
        let k_y = geo.k_y_at(0, 0); // (0,0)-(0,1)
        assert_eq!(k_x, 0.0, "x-edge across the diagonal corner should carry zero conveyance");
        assert_eq!(k_y, 0.0, "y-edge across the diagonal corner should carry zero conveyance");

        // And the fourth tile pairing across the corner, (1,0)-(0,1), isn't even adjacent in this
        // 2x2 coarse grid (no shared edge — diagonal tiles), so there is no k[e] for it at all.
        // That absence IS the point: a coarse graph over orthogonal edges has structurally no way
        // to represent a diagonal-only connection, matching what the fine solver itself can move.
    }

    /// Test 3 (task spec): conveyance of a real neck. At 512 the hourglass neck is ~5 cells wide
    /// against a t=8 boundary, so §4 estimates k ~= 5/8 = 0.625. Assert the MEASURED k at the
    /// narrowest coarse boundary is a sensible fraction — strictly between 0 and 1 — not that it
    /// hits 0.625 exactly (the estimate is illustrative, not a promise about which two rows a
    /// discrete t=8 tile boundary actually lands on around the neck).
    #[test]
    fn hourglass_neck_conveyance_at_512_is_a_sensible_fraction() {
        let grid = 512usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.generate_shape_mask();
        assert!(sim.coarse.available);
        let geo = &sim.coarse;
        let n = geo.coarse_n;
        let center_cx = n / 2; // neck is centered on the grid's x midline

        // Scan the y-edges near the vertical center for the tightest (minimum) conveyance --
        // the neck is a single narrow horizontal cross-section at the grid's vertical midline,
        // so its coarse boundary should show up as a local minimum in k_y near cy = n/2.
        let mut min_k = f32::INFINITY;
        let mut min_cy = 0usize;
        let lo = (n / 2).saturating_sub(4);
        let hi = (n / 2 + 4).min(n - 1);
        for cy in lo..hi {
            let k = geo.k_y_at(center_cx, cy);
            if k < min_k {
                min_k = k;
                min_cy = cy;
            }
        }

        eprintln!(
            "hourglass@512 neck: min k_y = {min_k} at cy={min_cy} (center_cx={center_cx}, t={})",
            geo.t
        );
        assert!(
            min_k > 0.0 && min_k < 1.0,
            "hourglass neck at 512 should be a partial, not binary, coarse edge: got k={min_k}"
        );
    }

    /// Test 4 (task spec): `MultiStageHourglass` at grid 128 (t=2), where
    /// `multistage_neck_half_width` floors the narrowest tier's neck at 1 cell. Measure the
    /// resulting k at that neck's coarse boundary and assert on the measured value (see report
    /// for the number and what it implies about a 1-cell neck against a 2-cell tile boundary).
    #[test]
    fn multistage_hourglass_floored_neck_conveyance_at_128() {
        let grid = 128usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::MultiStageHourglass;
        sim.generate_shape_mask();
        assert!(sim.coarse.available);
        assert_eq!(sim.coarse.t, 2, "grid 128 / COARSE_GRID 64 should give t=2");
        let geo = &sim.coarse;
        let n = geo.coarse_n;

        // MultiStageHourglass has MULTIPLE chambers per tier, each with its own LOCAL neck
        // x-position -- unlike plain Hourglass, these are not all under the global center_cx, so
        // the search must cover every (cx, cy) y-edge, not just one column. Track the global
        // minimum nonzero k_y (the tightest partial edge anywhere) as well as whether ANY edge
        // reads exactly 0 with an inside tile on both sides at that column (a fully-vanished
        // neck -- the case §4 warns about) versus a fractional edge (the floor being represented,
        // even if imperfectly).
        let mut min_k = f32::INFINITY;
        let mut min_loc = (0usize, 0usize);
        let mut fractional_edges = 0usize;
        for cy in 0..n.saturating_sub(1) {
            for cx in 0..n {
                let k = geo.k_y_at(cx, cy);
                if k > 0.0 && k < 1.0 {
                    fractional_edges += 1;
                }
                if k > 0.0 && k < min_k {
                    min_k = k;
                    min_loc = (cx, cy);
                }
            }
        }

        eprintln!(
            "multistage_hourglass@128 global tightest nonzero k_y = {min_k} at (cx,cy)={min_loc:?} \
             (t={}), {fractional_edges} fractional (0<k<1) y-edges total",
            geo.t
        );
        // Whatever the measured value is, it must be a valid conveyance fraction.
        assert!(min_k.is_finite() && min_k > 0.0 && min_k <= 1.0);
    }

    /// §9 step 1's requested measurement: how often does a shape's narrowest cross-section
    /// (a "neck", found as a local minimum in per-row open-cell count) fall STRICTLY INSIDE a
    /// coarse tile — i.e. neither the tile's first nor last fine row — rather than adjacent to a
    /// tile boundary where at least one coarse edge can represent it at all? §4 predicts roughly
    /// `(t-1)/t` of the time the constriction disappears from the coarse model entirely.
    ///
    /// `#[ignore]`d diagnostic, not a correctness assertion — run with
    /// `cargo test -p sandart-sim --lib --release coarse::tests::diag_neck_inside_tile_fraction -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn diag_neck_inside_tile_fraction() {
        for &grid in &[128usize, 256, 512] {
            for &shape in &[SandboxShape::Hourglass, SandboxShape::MultiStageHourglass] {
                let mut sim = DrawingSimulation::new_with_size(grid);
                sim.sandbox_shape = shape;
                sim.generate_shape_mask();
                if !sim.coarse.available {
                    eprintln!("grid={grid} shape={shape:?}: coarse unavailable, skipping");
                    continue;
                }
                let t = sim.coarse.t;

                // Per-row open-cell count over the whole fine mask.
                let w = grid;
                let row_width = |y: usize| -> usize {
                    (0..w)
                        .filter(|&x| sim.shape_mask[y * w + x] != MASK_OUTSIDE)
                        .count()
                };
                let widths: Vec<usize> = (0..grid).map(row_width).collect();

                // Local minima: width(y) <= both neighbors, and strictly less than at least one
                // (to exclude flat interiors), and nonzero (inside the shape at all).
                let mut necks_total = 0usize;
                let mut necks_inside_tile = 0usize;
                for y in 1..grid - 1 {
                    let wv = widths[y];
                    if wv == 0 {
                        continue;
                    }
                    let is_min = wv <= widths[y - 1]
                        && wv <= widths[y + 1]
                        && (wv < widths[y - 1] || wv < widths[y + 1]);
                    if !is_min {
                        continue;
                    }
                    necks_total += 1;
                    let pos_in_tile = y % t;
                    let on_edge = pos_in_tile == 0 || pos_in_tile == t - 1;
                    if !on_edge {
                        necks_inside_tile += 1;
                    }
                }

                let frac = if necks_total > 0 {
                    necks_inside_tile as f64 / necks_total as f64
                } else {
                    f64::NAN
                };
                eprintln!(
                    "grid={grid:>4} shape={shape:?}: t={t}, local-minima rows={necks_total}, \
                     inside-tile={necks_inside_tile}, fraction={frac:.3} \
                     (design predicts ~(t-1)/t = {:.3})",
                    (t - 1) as f64 / t as f64
                );
            }
        }
    }

    /// Sanity: grid 64 (COARSE_GRID itself) is the degenerate case and must come back
    /// `available = false`, not silently build a `t=1` geometry that would double-count.
    #[test]
    fn grid_64_is_unavailable_not_degenerate() {
        let mut sim = DrawingSimulation::new_with_size(64);
        sim.sandbox_shape = SandboxShape::Circle;
        sim.generate_shape_mask();
        assert!(!sim.coarse.available, "grid 64 == COARSE_GRID should be unavailable (t=1)");
        assert_eq!(sim.coarse.t, 0);
        // Still correctly sized, just inert.
        assert_eq!(sim.coarse.open_cells.len(), COARSE_GRID * COARSE_GRID);
    }

    /// `capacity` can be refreshed independently of geometry: rebuilding it twice with different
    /// wetness should change `capacity` but leave `open_cells`/`k_x`/`k_y` untouched.
    #[test]
    fn refresh_capacity_does_not_touch_geometry() {
        let grid = 128usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Circle;
        sim.generate_shape_mask();
        assert!(sim.coarse.available);

        let open_cells_before = sim.coarse.open_cells.clone();
        let k_x_before = sim.coarse.k_x.clone();
        let k_y_before = sim.coarse.k_y.clone();
        let capacity_before = sim.coarse.capacity.clone();

        // Bump every cell's wetness (liquid) and refresh capacity only.
        for chunk in sim.cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = 1.0;
        }
        let shape_mask = sim.shape_mask.clone();
        let cell_props = sim.cell_props.clone();
        sim.coarse.refresh_capacity(&shape_mask, &cell_props);

        assert_eq!(sim.coarse.open_cells, open_cells_before, "geometry must not change");
        assert_eq!(sim.coarse.k_x, k_x_before, "edges must not change");
        assert_eq!(sim.coarse.k_y, k_y_before, "edges must not change");
        assert_ne!(
            sim.coarse.capacity, capacity_before,
            "capacity should change: DrySand (1.5/cell) -> full liquidity (1.0/cell)"
        );
    }

    /// The struct is rebuilt at every `generate_shape_mask()` call site and nowhere else in this
    /// build step: changing the shape and regenerating should change `coarse`, and nothing reads
    /// it, so this is really just confirming the wiring exists and produces a different result.
    #[test]
    fn coarse_geometry_rebuilds_when_shape_mask_regenerates() {
        let mut sim = DrawingSimulation::new_with_size(128);
        sim.sandbox_shape = SandboxShape::Circle;
        sim.generate_shape_mask();
        let circle_open: u32 = sim.coarse.open_cells.iter().sum();

        sim.sandbox_shape = SandboxShape::Square;
        sim.generate_shape_mask();
        let square_open: u32 = sim.coarse.open_cells.iter().sum();

        // A square inscribed the same way as the circle covers more area, so this should differ.
        assert_ne!(circle_open, square_open);
    }

    /// Step 2 unit test: restriction preserves total mass exactly across all tiles.
    #[test]
    fn restriction_preserves_total_mass() {
        let grid = 128usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Circle;
        sim.generate_shape_mask();
        assert!(sim.coarse.available);

        // Fill circle with a gradient of fine mass
        let mut fine_mass_total = 0.0f32;
        for y in 0..grid {
            for x in 0..grid {
                let idx = y * grid + x;
                if sim.shape_mask[idx] != MASK_OUTSIDE {
                    let val = (x + y) as f32 * 0.01;
                    sim.heightmap.data[idx] = val;
                    fine_mass_total += val;
                }
            }
        }

        let mut state = CoarseState::new(sim.coarse.coarse_n);
        state.restrict(&sim.heightmap.data, &sim.shape_mask, &sim.cell_props, &sim.coarse);

        let coarse_mass_total: f32 = state.a_mass.iter().sum();
        let diff = (fine_mass_total - coarse_mass_total).abs();
        let rel_err = diff / fine_mass_total;
        assert!(
            rel_err < 1e-4,
            "restriction lost mass: fine={fine_mass_total}, coarse={coarse_mass_total}, diff={diff}, rel_err={rel_err}"
        );
    }

    /// Step 2 unit test: anchoring pulls M toward A by exactly lambda.
    #[test]
    fn anchor_pulls_coarse_mass_toward_fine_mass() {
        let grid = 128usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Square;
        sim.generate_shape_mask();

        let mut state = CoarseState::new(sim.coarse.coarse_n);
        state.lambda = 0.25;

        // Pick an inside tile
        let target_idx = sim.coarse.idx(sim.coarse.coarse_n / 2, sim.coarse.coarse_n / 2);
        assert!(sim.coarse.inside[target_idx], "target tile must be inside");

        state.a_mass[target_idx] = 10.0;
        state.m_mass[target_idx] = 0.0;

        state.anchor(&sim.coarse);
        assert!(
            (state.m_mass[target_idx] - 2.5).abs() < 1e-5,
            "M should be pulled to 2.5: got {}",
            state.m_mass[target_idx]
        );

        state.anchor(&sim.coarse);
        assert!(
            (state.m_mass[target_idx] - 4.375).abs() < 1e-5,
            "M should be pulled to 4.375: got {}",
            state.m_mass[target_idx]
        );
    }

    /// Step 2 unit test: coarse relaxation transfers mass downward under gravity and levels eta.
    #[test]
    fn coarse_relaxation_propagates_hydrostatic_head_downward() {
        let grid = 128usize;
        let mut sim = DrawingSimulation::new_with_size(grid);
        sim.sandbox_shape = SandboxShape::Square;
        sim.generate_shape_mask();

        let mut state = CoarseState::new(sim.coarse.coarse_n);
        let base_head = 1.0f32;
        let unit = 50.0f32;
        let n = sim.coarse.coarse_n;
        let cx = n / 2;

        // Find range of cy where tiles in column cx are inside
        let mut cy_inside = Vec::new();
        for cy in 0..n {
            let idx = cy * n + cx;
            if sim.coarse.inside[idx] && sim.coarse.capacity[idx] > 0.0 {
                cy_inside.push(cy);
                state.m_mass[idx] = sim.coarse.capacity[idx]; // exactly 1.0 nominal fill
            }
        }
        assert!(cy_inside.len() >= 4, "must have at least 4 inside tiles in column");
        let cy_top = *cy_inside.first().unwrap();
        let cy_bot = *cy_inside.last().unwrap();
        let idx_top = cy_top * n + cx;
        let idx_bot = cy_bot * n + cx;

        // This test drives `relax`/`update_head_and_disagreement` directly, bypassing `restrict`
        // -- so `support_mass` (the I5 deadband's gate, see that field's doc comment) is never
        // populated from real fine heights the way `tick()` would populate it. The bottom tile of
        // a closed column IS genuinely supported (it rests on the container floor), so register
        // that directly, mirroring what `restrict` would find if this were driven by real fine
        // data resting at the base of the column.
        state.support_mass[idx_bot] = 1.0;
        state.compute_grounded(&sim.coarse);

        let m_top_initial = state.m_mass[idx_top];
        let m_bot_initial = state.m_mass[idx_bot];

        // Relax for 64 sweeps
        state.relax(64, base_head, &sim.coarse, unit);
        state.update_head_and_disagreement(base_head, &sim.coarse, unit);

        let m_top_after = state.m_mass[idx_top];
        let m_bot_after = state.m_mass[idx_bot];

        assert!(
            m_bot_after > m_bot_initial,
            "bottom tile mass should increase under gravity: initial={m_bot_initial}, after={m_bot_after}"
        );
        assert!(
            m_top_after < m_top_initial,
            "top tile mass should decrease under gravity: initial={m_top_initial}, after={m_top_after}"
        );
        assert!(
            state.p_coarse[idx_bot] > 0.0,
            "bottom tile should have positive hydrostatic pressure"
        );
    }
}
