//! Coarse pressure geometry and state — HIERARCHICAL-PRESSURE.md, coupled into `settle_tick`
//! since build step 3, rebuilt per STEP4-COARSE-IS-A-SIM.md (2026-08-19).
//!
//! `CoarseGeometry` is a data structure derived once from `shape_mask` (and, for `capacity`/
//! `avg_wetness`, from `cell_props` wetness), rebuilt only where
//! `DrawingSimulation::generate_shape_mask` is rebuilt, tested against a flood fill of the fine
//! mask.
//!
//! `CoarseState` is the coarse level's own dynamics. Per the user's directive — *"let's just make
//! coarse sim same is 64x64"* — those dynamics are now the SHIPPED solver, `physics::settle_tick`,
//! run once per tick over a nested `coarse_n` x `coarse_n` grid (`NestedSim`, defined below;
//! `advance_nested_sim` is the entry point). What is NOT run through the shipped solver — because
//! it has no counterpart in a standalone simulation — is restriction (`A[C]`, aggregated from the
//! fine grid every tick) and anchoring (`M[C] += lambda * (A[C] - M[C])`, §0.4's "the memory lives
//! in `M`"); `NestedSim`'s own doc comment enumerates every further point where the coarse level
//! cannot be made bit-identical to a real 64x64 `DrawingSimulation`, and why.
//!
//! Also per user directive (previously "adaptive": STEP3-ADAPTIVE-COARSE.md), the coarse level
//! now runs exactly ONE step per tick, unconditionally — no sweep ceiling, no per-region adaptive
//! sweeping. That reading was a misapplication of the user's "adaptive" ruling, which was about
//! the FINE level's future over/underclocking, not the coarse level's own intra-tick iteration
//! count.
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

use crate::grid::Heightmap;
use crate::physics::{self, cell_capacity_for, overfill_pressure_val, ActiveBounds, ActiveMarbleInfo};
use crate::{
    BlockActivity, MASK_INSIDE, MASK_OUTSIDE, PROP_FLOW_RATE, PROP_GRAIN_SIZE, PROP_THRESHOLD,
    PROP_WETNESS,
};
use glam::Vec2;

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
    /// `avg_wetness[C]`: the arithmetic mean of `wetness` over the tile's open fine cells (0 for
    /// an empty tile). STEP4-COARSE-IS-A-SIM.md's divergence: the coarse level's own dynamics now
    /// run the real solver (`physics::settle_tick`) at 64x64, which derives a cell's capacity from
    /// a single `wetness` property, not from an externally supplied sum. `avg_wetness[C]` is the
    /// synthetic per-tile wetness handed to that nested solver so it can compute its own
    /// `cell_capacity_for` internally, matching the real code path as closely as a *sum* of
    /// (possibly heterogeneous) fine capacities can be represented by *one* wetness value.
    /// `cell_capacity_for` is linear in `liquidity(wetness)` but `liquidity` itself is a nonlinear
    /// (smoothstep) function of `wetness`, so `cell_capacity_for(avg_wetness[C])` equals
    /// `capacity[C] / open_cells[C]` (the true mean per-cell capacity) EXACTLY whenever a tile's
    /// fine cells share one wetness value (the common case — most scenes are single-material) and
    /// only approximately at a material boundary that a tile straddles. Refreshed on the same
    /// cadence as `capacity` (see that field's doc comment) since it comes from the same scan.
    pub avg_wetness: Vec<f32>,
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
            avg_wetness: vec![0.0f32; coarse_n * coarse_n],
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
        self.avg_wetness.iter_mut().for_each(|v| *v = 0.0);
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
                    self.avg_wetness[cy * n + cx] += wetness;
                }
            }
        }
        for c in 0..self.avg_wetness.len() {
            if self.open_cells[c] > 0 {
                self.avg_wetness[c] /= self.open_cells[c] as f32;
            }
        }
    }
}

/// Default anchoring rate lambda for pulling M towards A: 0.10 (§5 step 2).
pub const COARSE_DEFAULT_LAMBDA: f32 = 0.10;

/// The coarse level's own dynamics — literally a persistent `coarse_n` x `coarse_n`
/// `DrawingSimulation`-shaped sandbox, run one `physics::settle_tick` per coarse tick, per the
/// user's directive (STEP4-COARSE-IS-A-SIM.md): *"let's just make coarse sim same is 64x64."*
///
/// `settle_tick` is the shipped solver's entry point; nothing here reimplements the overfill law,
/// the fill-fraction/pressure split, or the flux solver — this struct only owns the buffers that
/// entry point needs, sized to the coarse grid instead of the fine one, and persistent across
/// ticks exactly like `DrawingSimulation`'s own buffers are (this is what makes edge momentum
/// (`edge_vel_h`/`edge_vel_v`) and the LOD scheduler's own bookkeeping carry over tick to tick,
/// matching a *simulation* rather than a solve run fresh each time).
///
/// **Documented divergences from a real `DrawingSimulation` at `grid_size = coarse_n`** (per the
/// task's own instruction: "every place you depart from that is a place you owe an explanation"):
///
/// 1. **The height fed in and read back is an AVERAGE, not the tile's total mass.** `M[C]`
///    (`CoarseState::m_mass`) is a SUM over the tile's `t x t` fine cells (restriction has no
///    counterpart in a standalone sim — see `CoarseState::restrict`'s own doc comment) but
///    `settle_tick`'s `cell_capacity_for` and overfill law are calibrated per FINE cell (capacity
///    ~1.0-1.5). `advance_nested_sim` therefore feeds `M[C] / open_cells[C]` in and multiplies the
///    result back out by `open_cells[C]` afterward — the nested sim's own cells behave exactly
///    like real 64x64 cells; only the SCALE of the value handed to/from `CoarseState` differs.
/// 2. **Capacity comes from one synthetic `wetness` per tile (`CoarseGeometry::avg_wetness`), not
///    an externally injected capacity.** `settle_tick` derives a cell's capacity internally from
///    its own `wetness` property; there is no parameter to hand it `capacity[C]` directly. See
///    `avg_wetness`'s own doc comment for the exact/approximate cases.
/// 3. **No LOD scheduling within the coarse tick.** `block_size` is set to the whole grid (one
///    block) and that block's `last_displacements` is forced above the MUST-simulate threshold
///    before every call, so every coarse cell gets a full step every coarse tick — matching the
///    user's directive 1 ("one step per tick, not adaptive") and avoiding a second, nested LOD
///    scheduler with its own staleness/budget semantics layered on top of the outer one.
/// 4. **No marbles, no head-field toggle, no coarse-of-coarse coupling.** `active_marbles = &[]`,
///    `head_field_transport`/`pressure_heatmap_head_field`/`pressure_sensitive_flow = false`, and
///    `coarse_eta`/`coarse_delta = &[]` — the coarse level is not itself coupled to a third,
///    still-coarser level. `fresh_pressure_field` is left `false` too (the production default),
///    so the nested sim's `column_depth` behaviour matches the shipped, non-toggled fine sim.
/// 5. **`cell_props`' non-wetness fields are fixed defaults** (`PROP_THRESHOLD = 0.08`,
///    `PROP_FLOW_RATE = 0.25`, `PROP_GRAIN_SIZE = 0.45` — `DrawingSimulation::new_with_size`'s own
///    defaults), not aggregated from the fine grid, since the overfill path this design couples
///    through does not read them; they exist only so `settle_tick`'s legacy (non-overfill)
///    branches see sane values if ever reached.
#[derive(Debug, Clone)]
struct NestedSim {
    heightmap: Heightmap,
    temp_heights: Vec<f32>,
    cell_colors: Vec<u8>,
    cell_props: Vec<f32>,
    sliding: Vec<bool>,
    active_bounds: ActiveBounds,
    active_blocks: Vec<BlockActivity>,
    last_displacements: Vec<f32>,
    last_simulated_ticks: Vec<u32>,
    edge_vel_h: Vec<f32>,
    edge_vel_v: Vec<f32>,
    column_depth: Vec<f32>,
    head_field: Vec<f32>,
    shape_mask: Vec<u8>,
    tick_count: u32,
}

impl NestedSim {
    fn new(coarse_n: usize) -> Self {
        let size = coarse_n * coarse_n;
        let mut cell_props = vec![0.0f32; size * 4];
        for chunk in cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = 0.0;
            chunk[PROP_THRESHOLD] = 0.08;
            chunk[PROP_FLOW_RATE] = 0.25;
            chunk[PROP_GRAIN_SIZE] = 0.45;
        }
        Self {
            heightmap: Heightmap::new(coarse_n, coarse_n, 0.0),
            temp_heights: vec![0.0f32; size],
            cell_colors: vec![0u8; size * 4],
            cell_props,
            sliding: vec![false; size],
            active_bounds: ActiveBounds { min_x: 0, max_x: 0, min_y: 0, max_y: 0, active: false },
            active_blocks: vec![BlockActivity::Inactive; 1],
            last_displacements: vec![0.0f32; 1],
            last_simulated_ticks: vec![0u32; 1],
            edge_vel_h: vec![0.0f32; size],
            edge_vel_v: vec![0.0f32; size],
            column_depth: vec![0.0f32; size],
            head_field: vec![0.0f32; size],
            shape_mask: vec![MASK_OUTSIDE; size],
            tick_count: 0,
        }
    }

    /// Resize every buffer to `coarse_n`, e.g. after a geometry rebuild that changes `coarse_n`
    /// (production never does — `coarse_n` is always `COARSE_GRID` — but the unit tests below
    /// build smaller geometries). A resize is a fresh start (zeroed state, `tick_count = 0`):
    /// there is no meaningful way to resample a persistent simulation onto a differently-sized
    /// grid, and this happens only via `CoarseState::new`, which is itself a fresh start for
    /// everything else in `CoarseState` too.
    fn resize(&mut self, coarse_n: usize) {
        *self = Self::new(coarse_n);
    }
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
    /// Anchor strength lambda in (0, 1].
    pub lambda: f32,
    /// The coarse level's own dynamics, literally: a persistent nested 64x64 (or `coarse_n` x
    /// `coarse_n`, for the smaller grids some unit tests build) simulation, run one
    /// `physics::settle_tick` per coarse tick — see `NestedSim` and `advance_nested_sim`.
    nested: NestedSim,
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
            lambda: COARSE_DEFAULT_LAMBDA,
            nested: NestedSim::new(coarse_n),
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

    /// Incremental version of `restrict`, exact rather than approximate: a block that was
    /// neither simulated nor received flux this tick has bit-identically the same fine heights
    /// it had when `A[C]`/`support_mass[C]` were last aggregated for it, so re-summing it is
    /// provably a no-op -- it can simply be skipped. `restrict`'s own full-grid scan measured
    /// ~7.8ms of fixed per-tick cost at grid 512 (STEP3-FIXES.md's "found beyond the three
    /// defects"); this touches only the tiles that could possibly have changed.
    ///
    /// `touched[b]`: one bool per BLOCK, `true` iff block `b` was simulated
    /// (`will_simulate[b]`) or marked `modified` by flux from a running neighbour
    /// (`activate_neighbor`/`flux_edge_apply` -- existing, load-bearing scheduler machinery, see
    /// `physics.rs::settle_tick`'s `touched_out` parameter, which is exactly this array).
    /// **Block index and coarse tile index are the same integer**: `block_size == grid_size/64
    /// == COARSE_GRID` at every resolution where the coarse level is available (both require
    /// exact division by 64), so a block's `(bx, by)` and a coarse tile's `(cx, cy)` coincide
    /// one-to-one -- no partial-tile bookkeeping, per the task's own framing. Verified bit-exact
    /// against a full `restrict` over a long run of every shipped shape by
    /// `incremental_restrict_is_bit_exact_to_full_rebuild` below.
    ///
    /// Falls back to a full `restrict` if `touched.len()` does not match `coarse_n * coarse_n`
    /// (e.g. the very first tick after construction/a mask rebuild, before any `settle_tick` has
    /// run to populate a real touched set -- see `DrawingSimulation::blocks_touched`'s own doc
    /// comment for when that happens) -- correctness over the optimisation whenever the caller
    /// cannot vouch for the touched set.
    pub fn restrict_incremental(
        &mut self,
        heights: &[f32],
        shape_mask: &[u8],
        cell_props: &[f32],
        geo: &CoarseGeometry,
        touched: &[bool],
    ) {
        if !geo.available || heights.len() != geo.grid_size * geo.grid_size || shape_mask.len() != heights.len()
            || cell_props.len() != heights.len() * 4 {
            return;
        }
        if touched.len() != self.a_mass.len() {
            self.restrict(heights, shape_mask, cell_props, geo);
            return;
        }
        let t = geo.t;
        let n = self.coarse_n;
        let g = geo.grid_size;
        for cy in 0..n {
            let y0 = cy * t;
            let y1 = ((cy + 1) * t).min(g);
            for cx in 0..n {
                let ci = cy * n + cx;
                if !touched[ci] {
                    continue;
                }
                let x0 = cx * t;
                let x1 = ((cx + 1) * t).min(g);
                let mut a = 0.0f32;
                let mut sup = 0.0f32;
                for fy in y0..y1 {
                    let row = fy * g;
                    for fx in x0..x1 {
                        let fi = row + fx;
                        if shape_mask[fi] != MASK_OUTSIDE {
                            let h = heights[fi];
                            a += h;
                            let cap = cell_capacity_for(cell_props[fi * 4 + PROP_WETNESS]);
                            if h > cap {
                                sup += h - cap;
                            }
                        }
                    }
                }
                self.a_mass[ci] = a;
                self.support_mass[ci] = sup;
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

    /// Step 3 of §5, rebuilt per STEP4-COARSE-IS-A-SIM.md (directives 1 and 2): advance the
    /// coarse level's own state by running the SHIPPED solver, `physics::settle_tick`, once over
    /// the nested `coarse_n` x `coarse_n` grid -- literally the same code path a real
    /// `DrawingSimulation` at that size would call every tick, not a bespoke relaxation that
    /// resembles it. See `NestedSim`'s doc comment for the exact, enumerated divergences (height
    /// fed in as an average not a sum, capacity from one synthetic wetness, no LOD scheduling,
    /// no marbles/head-field/deeper coupling).
    ///
    /// **One call is one coarse tick (directive 1).** The previous adaptive/ceiling sweep
    /// machinery (`COARSE_DEFAULT_SWEEPS`, `ACTIVE_DELTA_REL_TOL`, per-region active-tile
    /// sweeping) is gone: the user's "adaptive" ruling was about the FINE level's future
    /// over/underclocking, not the coarse level, which "we only need one... simulation per
    /// tick" of (§0.4's own words: `M` persists, so it only needs to ADVANCE, not converge
    /// within a tick -- exactly like the fine level advances one step per tick, no more).
    ///
    /// **`gravity_dir` is the real simulation's own gravity vector, unscaled.** This is
    /// directive 2's dynamics fix: the coarse level's own per-row head is
    /// `gravity_dir.y * GRAVITY_HEAD_SCALE`, computed internally by `settle_tick` exactly as the
    /// fine level computes it -- "one row per row", not `base_head * t`. The previous bespoke
    /// `base_head_coarse = base_head * geo.t` scaling (`t = 8` at grid 512) is what drove the
    /// 9x over-drive this rebuild fixes; see `update_head_and_disagreement` for the matching
    /// export-side fix and STEP4-COARSE-IS-A-SIM.md for the measurement.
    pub fn advance_nested_sim(
        &mut self,
        geo: &CoarseGeometry,
        gravity_dir: Vec2,
        overfill_ratio: f32,
        underfill_tension: f32,
        overfill_stiffness: f32,
    ) {
        if !geo.available {
            return;
        }
        let n = self.coarse_n;
        if self.nested.heightmap.width != n {
            self.nested.resize(n);
        }
        // Geometry/wetness -> nested sim inputs. Rebuilt every call (cheap, <= COARSE_GRID^2 =
        // 4096 cells) rather than cached against a geometry-change flag, for simplicity; nothing
        // here is on a path sensitive enough to that cost to justify the extra invalidation
        // bookkeeping.
        for c in 0..n * n {
            self.nested.shape_mask[c] = if geo.inside[c] { MASK_INSIDE } else { MASK_OUTSIDE };
            self.nested.cell_props[c * 4 + PROP_WETNESS] = geo.avg_wetness[c];
            // NestedSim divergence 1: M[C] is a SUM over the tile's fine cells; the nested sim's
            // own cells behave like real per-fine-cell cells (capacity ~1.0-1.5), so the height
            // fed in is the AVERAGE, converted back to a sum after the step below.
            self.nested.heightmap.data[c] = if geo.open_cells[c] > 0 {
                self.m_mass[c] / geo.open_cells[c] as f32
            } else {
                0.0
            };
        }
        // NestedSim divergence 3: force the (single) block to MUST-simulate every call, so this
        // one `settle_tick` call is a full, unconditional step over the whole coarse grid --
        // directive 1's "one step per tick", not partial/adaptive.
        self.nested.last_displacements.iter_mut().for_each(|v| *v = 1.0);
        let block_size = n.max(1);
        const NESTED_BUDGET_N: usize = 1_000_000;
        physics::settle_tick(
            &mut self.nested.heightmap,
            &mut self.nested.temp_heights,
            &mut self.nested.cell_colors,
            &mut self.nested.cell_props,
            &mut self.nested.sliding,
            &mut self.nested.active_bounds,
            &mut self.nested.active_blocks,
            &mut self.nested.last_displacements,
            &mut self.nested.last_simulated_ticks,
            NESTED_BUDGET_N,
            block_size,
            &[] as &[ActiveMarbleInfo],
            self.nested.tick_count,
            &mut self.nested.edge_vel_h,
            &mut self.nested.edge_vel_v,
            &mut self.nested.column_depth,
            &mut self.nested.head_field,
            &self.nested.shape_mask,
            self.nested.tick_count,
            gravity_dir,
            false, // fresh_pressure_field
            false, // head_field_transport
            false, // pressure_heatmap_head_field
            false, // pressure_sensitive_flow
            true,  // overfill_pressure -- the coarse level always runs #70's overfill law
            overfill_ratio,
            underfill_tension,
            overfill_stiffness,
            &[], // coarse_eta -- no coarse-of-coarse coupling
            &[], // coarse_delta
            None, // precomputed_fresh_active (Stage 1 hoist): the coarse level's own nested tick recomputes internally, not part of the fine-level overclocking loop
            None, // touched_out
        );
        self.nested.tick_count = self.nested.tick_count.wrapping_add(1);
        // Read back: average -> sum (the inverse of the feed-in above).
        for c in 0..n * n {
            self.m_mass[c] = if geo.inside[c] {
                self.nested.heightmap.data[c] * geo.open_cells[c] as f32
            } else {
                0.0
            };
        }
    }

    /// Step 4 of §5: read back `eta`, `P`, and `Delta` from the nested sim's post-tick state.
    ///
    /// **Rebuilt per STEP4-COARSE-IS-A-SIM.md.** The old I5 deadband (`support_mass`-gated
    /// `grounded[C]`, withholding the pressure-excess term above capacity for an "ungrounded"
    /// tile) is REMOVED: it existed only because the previous bespoke relax could develop
    /// unsupported compression a hand-rolled rule had to suppress. `advance_nested_sim` now runs
    /// the real overfill solver, which enforces "compression is support" (§3) the same way it
    /// already does at the fine level -- structurally, via the flux solver itself, not a second
    /// rule layered on top. `support_mass`/`restrict`'s aggregation of it is UNCHANGED (kept
    /// per-task instruction, along with its bit-exactness test) but is no longer consumed here.
    ///
    /// `phi`/`x` reuse the real solver's own function (`overfill_pressure_val`) rather than a
    /// bespoke linear approximation (the old
    /// `coarse_unbounded_phi`'s excess term was LINEAR in `x`; the real law is quadratic in the
    /// overfill fraction via `overfill_ratio` -- see `overfill_pressure_val`'s doc comment). This
    /// is computed from `cell_capacity_for(geo.avg_wetness[c])`, matching exactly what
    /// `advance_nested_sim` fed the nested solver for that same cell, and `unit` is the FINE
    /// grid's own `overfill_head_unit` (passed in by the caller, `DrawingSimulation::update`) so
    /// the pressure-excess term this exports is in the SAME units the fine coupling
    /// (`physics::coarse_delta_eta`) already expects -- unchanged from before this rebuild.
    ///
    /// **The elevation term is `- cy * base_head_coarse` with `base_head_coarse =
    /// gravity_dir.y * GRAVITY_HEAD_SCALE` -- the SAME per-row head the nested sim's own vertical
    /// pass just used, "one row per row" (directive 2), not `base_head * geo.t`.** This is the
    /// fix for the over-drive: `eta` is `phi` with elevation netted out (§0.3: constant down a
    /// resting column), and now the elevation it nets out is stated at the SAME scale the fine
    /// level's own `base_head` uses, rather than at `t`x that scale. See
    /// STEP4-COARSE-IS-A-SIM.md for the measured `delta_eta` this produces at a resting seam.
    pub fn update_head_and_disagreement(
        &mut self,
        geo: &CoarseGeometry,
        gravity_dir: Vec2,
        overfill_ratio: f32,
        underfill_tension: f32,
        unit: f32,
    ) {
        if !geo.available {
            self.eta.iter_mut().for_each(|v| *v = 0.0);
            self.p_coarse.iter_mut().for_each(|v| *v = 0.0);
            self.delta.iter_mut().for_each(|v| *v = 0.0);
            return;
        }
        let n = self.coarse_n;
        let base_head_coarse = gravity_dir.y * physics::GRAVITY_HEAD_SCALE;

        for cy in 0..n {
            for cx in 0..n {
                let c = cy * n + cx;
                let a = self.a_mass[c];
                self.delta[c] = self.m_mass[c] - a;
                let cap = cell_capacity_for(geo.avg_wetness[c]);
                let h = self.nested.heightmap.data[c];
                if cap > 0.0 && geo.inside[c] {
                    let x = h / cap;
                    let phi = if x <= 1.0 || unit <= 0.0 {
                        x
                    } else {
                        x + overfill_pressure_val(h, cap, overfill_ratio, unit, underfill_tension) / unit
                    };
                    self.eta[c] = phi - cy as f32 * base_head_coarse;
                    self.p_coarse[c] = (phi - x).max(0.0);
                } else {
                    // SMOOTH-ETA.md (build step, post-STEP4): `cap > 0.0` is unconditionally true
                    // whenever `geo.inside[c]` (`cell_capacity_for` returns 1.0..=1.5 for every
                    // wetness), so this branch is exactly `!geo.inside[c]` -- a coarse cell OUTSIDE
                    // the sandbox geometry: solid wall, not a dry-but-reachable pocket (that case is
                    // `inside[c] = true, h = 0`, handled above by the `x <= 1.0` branch with
                    // `eta = -cy * base_head_coarse`, the same "free surface at its own elevation"
                    // convention the fine solver already uses for a dry cell). Bilinear
                    // interpolation of `eta` onto the fine grid (`physics::eta_fine_interp`) must
                    // never blend a wall's `eta` into a wet cell's driving potential -- material
                    // cannot cross that wall, so there is no physical quantity for `eta` to
                    // represent there. NaN, not 0.0, marks this: 0.0 is a plausible genuine `eta`
                    // reading (e.g. exactly at a free surface at row 0), so it cannot double as
                    // "invalid" without silently corrupting a real one.
                    // `eta_fine_interp` filters NaN samples out of its 4-corner blend and
                    // renormalises weight over whatever in-mask corners remain; if every corner
                    // within interpolation reach of a fine cell is NaN (a wall-locked pocket, not
                    // reachable by any real fine cell in practice since a reachable fine cell's own
                    // tile is always `inside`), it returns NaN itself and `coarse_delta_eta` treats
                    // that as "no coupling on this edge" (delta 0.0), never propagating NaN further.
                    self.eta[c] = f32::NAN;
                    self.p_coarse[c] = 0.0;
                }
            }
        }
    }

    /// Full tick update (Step 1 -> Step 2 -> Step 3 -> Step 4).
    ///
    /// `touched`: `Some(blocks)` uses `restrict_incremental` (see that method's doc comment for
    /// the exactness argument and the block/tile index correspondence); `None` always does a
    /// full `restrict` (used by callers, e.g. tests, that have no touched set to vouch for).
    ///
    /// `gravity_dir`/`overfill_ratio`/`underfill_tension`/`overfill_stiffness` are the REAL
    /// simulation's own settings, unmodified -- see `advance_nested_sim`'s doc comment for why
    /// they are passed through as-is rather than rescaled for the coarse grid. `unit` is the
    /// FINE grid's own `overfill_head_unit` (`(GRAVITY_HEAD_SCALE / depth_scale) *
    /// overfill_stiffness`, computed by the caller from the FINE width), used only for
    /// `update_head_and_disagreement`'s export-side pressure scaling.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        heights: &[f32],
        shape_mask: &[u8],
        cell_props: &[f32],
        geo: &CoarseGeometry,
        gravity_dir: Vec2,
        overfill_ratio: f32,
        underfill_tension: f32,
        overfill_stiffness: f32,
        unit: f32,
        touched: Option<&[bool]>,
    ) {
        if !geo.available {
            return;
        }
        match touched {
            Some(t) => self.restrict_incremental(heights, shape_mask, cell_props, geo, t),
            None => self.restrict(heights, shape_mask, cell_props, geo),
        }
        self.anchor(geo);
        self.advance_nested_sim(geo, gravity_dir, overfill_ratio, underfill_tension, overfill_stiffness);
        self.update_head_and_disagreement(geo, gravity_dir, overfill_ratio, underfill_tension, unit);
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
        let gravity_dir = glam::Vec2::new(0.0, 0.04); // base_head = 0.04 * GRAVITY_HEAD_SCALE = 1.0
        let overfill_ratio = 0.5f32;
        let underfill_tension = 0.0f32;
        let overfill_stiffness = crate::physics::OVERFILL_STIFFNESS_K;
        let unit = 50.0f32;
        let n = sim.coarse.coarse_n;
        let cx = n / 2;

        // Find range of cy where tiles in column cx are inside
        let mut cy_inside = Vec::new();
        for cy in 0..n {
            let idx = cy * n + cx;
            if sim.coarse.inside[idx] && sim.coarse.capacity[idx] > 0.0 {
                cy_inside.push(cy);
                state.m_mass[idx] = sim.coarse.capacity[idx]; // exactly nominal (x=1) fill
            }
        }
        assert!(cy_inside.len() >= 4, "must have at least 4 inside tiles in column");
        let cy_top = *cy_inside.first().unwrap();
        let cy_bot = *cy_inside.last().unwrap();
        let idx_top = cy_top * n + cx;
        let idx_bot = cy_bot * n + cx;

        let m_top_initial = state.m_mass[idx_top];
        let m_bot_initial = state.m_mass[idx_bot];

        // This test drives `advance_nested_sim`/`update_head_and_disagreement` directly,
        // bypassing `restrict`/`anchor` (there is no fine data here to restrict from) -- 64
        // coarse ticks of the REAL nested solver, matching the old test's "relax for 64 sweeps"
        // in spirit: the coarse level is a simulation that advances one step per tick (directive
        // 1), so the equivalent of "let it relax" is "let it tick".
        for _ in 0..64 {
            state.advance_nested_sim(&sim.coarse, gravity_dir, overfill_ratio, underfill_tension, overfill_stiffness);
        }
        state.update_head_and_disagreement(&sim.coarse, gravity_dir, overfill_ratio, underfill_tension, unit);

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

    /// STEP3-ADAPTIVE-COARSE.md: `restrict_incremental` must be EXACT, never an approximation
    /// (task requirement). Drives the real production path -- `DrawingSimulation::update()`,
    /// which calls `CoarseState::tick` with `Some(&self.blocks_touched)` every tick -- for every
    /// shipped shape, and after every tick recomputes a FULL `restrict` from a snapshot of the
    /// exact fine state (`heightmap`/`shape_mask`/`cell_props`) taken immediately before that
    /// tick's `update()` call (the same state `coarse_state.tick()` itself reads, since it runs
    /// before `settle_tick` mutates heights -- see `lib.rs`'s call ordering), asserting
    /// `a_mass`/`support_mass` match the incremental result bit-for-bit (`==`, not a tolerance).
    #[test]
    fn incremental_restrict_is_bit_exact_to_full_rebuild() {
        let grid = 128usize;
        let ticks = 300usize;
        let targets = [None; 5];

        for &shape in &all_shapes() {
            let mut sim = DrawingSimulation::new_with_size(grid);
            sim.sandbox_shape = shape;
            sim.gravity_dir = glam::Vec2::new(0.0, 0.04);
            sim.apply_preset(crate::MaterialMode::Water);
            sim.overfill_pressure = true;
            // Self-sufficient (regenerates the mask) and generic across shapes: seeds material
            // at the top so restriction has real, changing mass to track, not an empty grid.
            sim.initialize_hourglass();
            assert!(sim.coarse.available, "{shape:?}: coarse should be available at grid {grid}");

            for tick in 0..ticks {
                let heights_before = sim.heightmap.data.clone();
                let mask_before = sim.shape_mask.clone();
                let props_before = sim.cell_props.clone();

                sim.update(0.016, &targets, 0.08, crate::MaterialMode::Water, shape, 16.0, 16.0);

                let mut full = CoarseState::new(sim.coarse.coarse_n);
                full.restrict(&heights_before, &mask_before, &props_before, &sim.coarse);

                assert_eq!(
                    sim.coarse_state.a_mass, full.a_mass,
                    "{shape:?} tick {tick}: incremental A[C] diverged from a full rebuild"
                );
                assert_eq!(
                    sim.coarse_state.support_mass, full.support_mass,
                    "{shape:?} tick {tick}: incremental support_mass[C] diverged from a full rebuild"
                );
            }
        }
    }
}
