pub mod grid;
pub mod physics;
pub mod quantiles;

pub use grid::Heightmap;
pub use physics::{ActiveBounds, displace_line, settle_tick};
pub use quantiles::{
    compute_quantile_positions, refresh_row_mass_active, refresh_row_mass_full, QuantileMode,
    DECILE_FRACTIONS, MAX_QUANTILE_LINES, QUARTILE_FRACTIONS,
};
use glam::Vec2;
use serde::{Deserialize, Serialize};

// PERF-PROFILE.md MEASUREMENT INSTRUMENTATION. Retained deliberately: it is the instrument
// that produced the "~59-62% of extra sub-steps run on already-settled blocks" finding, and
// it is how the early-termination fix will be verified once built. Measured non-perturbing --
// lib suite 102/10 unchanged, 115.6 ms/frame against 120.1 without it (noise), mass_err
// 1.29e-9. DELETE IT once early termination lands and has been re-measured. Records, per
// overclocked block per frame, (target sub-steps, first sub-step index at which the block's own
// physically-computed `last_displacements` fell under `MUST_SIMULATE_THRESHOLD`, or -1 if it
// never did). Read right after each `settle_tick` call, BEFORE `force_overclocked_blocks_active`
// overwrites `last_displacements` for the next repetition, so this sees the real number, not the
// forced floor. Answers Job 2 candidate 5's "how often does a block reach local equilibrium
// before its n sub-steps are done".
thread_local! {
    static EARLY_TERM_LOG: std::cell::RefCell<Vec<(u32, i32)>> = const { std::cell::RefCell::new(Vec::new()) };
}
pub const GRID_SIZE: usize = 512;
pub const DEFAULT_SAND_HEIGHT: f32 = 0.35;

/// Gravity magnitude for Sand-fall mode, shared by both front ends (`sandart/src/app.rs`'s
/// desktop build and `sandart-wasm/web/demo.js`'s hardcoded mirror — JS can't `use` this
/// constant, so keep the two in sync by hand if this ever changes).
///
/// This used to be a user-facing slider (`#gravity-slider`, range 0.04..=0.10 step 0.005). It was
/// removed after measuring both materials across that whole range and finding it flat: DrySand's
/// Hourglass upper-chamber drain time to 50% was bit-identical (143 ticks) at every step from 0.04
/// to 0.10, and the terminal free-fall speed of a dropped block pinned at exactly 1.0 rows/tick
/// for every gravity value tested, for both DrySand and Water. The reason is `flux_edge`'s per-tick
/// transfer clamp (`cell_capacity_for`: 1.5 granular / 1.0 liquid) — the driving head this gravity
/// magnitude produces (`g * GRAVITY_HEAD_SCALE`, see `physics.rs`) already exceeds that clamp at
/// g = 0.04, so raising g further only raises a quantity that is already being clipped every tick.
/// 0.06 keeps clear margin above the g >= 0.04 boiling threshold without being any more "correct"
/// than any other value in the measured range.
pub const SANDFALL_GRAVITY_STRENGTH: f32 = 0.06;

/// Shape mask cell values: the single source of truth for container geometry.
pub const MASK_OUTSIDE: u8 = 0;
pub const MASK_INSIDE: u8 = 1;
pub const MASK_BOUNDARY: u8 = 2;






/// The rate ladder the RANK rule fills (see `rank_clock_rates`), highest first. Integer steps
/// down to 1x and octaves below it: above 1x a rate IS a repetition count, so fractional values
/// there only round back onto these anyway (`extra_reps` rounds), while below 1x a rate is a
/// SKIP PERIOD, where octaves are the meaningful spacing.
///
/// Band sizes are `n_r ∝ 1/r`, normalised over whichever bands survive the
/// `min_clock_rate`/`max_clock_rate` clip, so each band performs the same total work and the
/// frame's whole block-step count is `bands / Σ(1/r)` times the participating block count --
/// 0.66x for the full ladder. That is the property that makes this a REPLACEMENT for a flat
/// scheduler rather than an addition to one: the fractional bands fund the 8x band.
/// The shipped LOD block geometry: a CONSTANT block edge of 8 cells, so the block grid is 8x8 at
/// grid 64, 16x16 at 128, 32x32 at 256 and 64x64 at 512.
///
/// This replaces the `block_size = grid/64` geometry, which pinned the block COUNT at 64x64 for
/// every resolution and let the block SIZE float (1 cell at grid 64, 2 at 128, 4 at 256). That
/// was chosen only to make a block and a coarse pressure tile the same square -- `COARSE_GRID`
/// was also 64 -- and the coarse level was deleted on 2026-08-30, so nothing justifies 64 any
/// more. Two things it cost, both now gone: `block_size = 1` at grid 64 degenerated the LOD
/// scheduler to one block per cell, and grid 128's `block_size = 2` shipped into the slab
/// artifact `VERTICAL_PRESSURE_CAP_MULT`'s doc comment documents.
///
/// At grid 512 -- the shipped `GRID_SIZE` -- this is a no-op: 512/8 = 64 blocks per axis, exactly
/// the geometry that was already running. Only the smaller grids change, and they change toward
/// the non-degenerate case.
///
/// The divisor is clamped to 64, so a future grid 1024 gets 64x64 blocks of 16 cells rather than
/// 128x128 of 8. That bound is a placeholder for a resolution that does not exist yet; revisit it
/// when 1024 is actually added rather than treating it as measured.
pub(crate) const DEFAULT_BLOCK_SIZE: usize = 8;
pub(crate) const MAX_BLOCKS_PER_AXIS: usize = 64;

/// Blocks per axis for a grid, i.e. the `divisor` in `block_size = grid/divisor`.
pub(crate) fn default_block_divisor(grid_size: usize) -> usize {
    (grid_size / DEFAULT_BLOCK_SIZE).clamp(1, MAX_BLOCKS_PER_AXIS)
}

/// The adaptive frame-time controller's four throttles, as FRACTIONS of the block count.
///
/// They used to be absolute block counts (1024 / 128 / 16 / 4), correct only while the block count
/// was pinned at 4096. Now that the count scales with the grid again they have to be derived, or a
/// small grid would inherit a floor that is most of its domain. The divisors below are chosen to
/// reproduce the previous absolute values EXACTLY at 4096 blocks (grid 512, the shipped size), so
/// this is a no-op at the default resolution and a rescale everywhere else.
///
/// Returns `(initial, min, step_down, step_up)`.
pub(crate) fn budget_throttles(block_count: usize) -> (usize, usize, usize, usize) {
    (
        (block_count / 4).max(1),    // 1024 at 4096
        (block_count / 32).max(1),   //  128 at 4096
        (block_count / 256).max(1),  //   16 at 4096
        (block_count / 1024).max(1), //    4 at 4096
    )
}






pub const PROP_WETNESS: usize = 0;
pub const PROP_THRESHOLD: usize = 1;
pub const PROP_FLOW_RATE: usize = 2;
pub const PROP_GRAIN_SIZE: usize = 3;

/// Per-cell physics & render properties, one `Vec<f32>` per channel (structure-of-arrays)
/// instead of the historical `[wetness, threshold, flow_rate, grain_size]`-interleaved
/// `Vec<f32>`. Pure storage-layout change -- see `to_interleaved`/`copy_from_interleaved` for
/// the historical layout, still used at the JS-facing `set_cell_props` boundary and by
/// diagnostics/dumpers that hash or persist that exact byte order. Channel order (0..=3) still
/// matches `PROP_WETNESS..PROP_GRAIN_SIZE`, so `get`/`set` stay a drop-in replacement for the old
/// `props[i * 4 + PROP_X]` indexing wherever the channel is a runtime variable rather than a
/// constant.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellProps {
    pub wetness: Vec<f32>,
    pub threshold: Vec<f32>,
    pub flow_rate: Vec<f32>,
    pub grain_size: Vec<f32>,
}

impl CellProps {
    pub fn new(len: usize) -> Self {
        Self {
            wetness: vec![0.0; len],
            threshold: vec![0.0; len],
            flow_rate: vec![0.0; len],
            grain_size: vec![0.0; len],
        }
    }

    /// All `len` cells set to the same four values -- the common case (a material preset
    /// applied uniformly).
    pub fn filled(len: usize, wetness: f32, threshold: f32, flow_rate: f32, grain_size: f32) -> Self {
        Self {
            wetness: vec![wetness; len],
            threshold: vec![threshold; len],
            flow_rate: vec![flow_rate; len],
            grain_size: vec![grain_size; len],
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.wetness.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.wetness.is_empty()
    }

    /// Read channel `ch` (0=wetness, 1=threshold, 2=flow_rate, 3=grain_size) of cell `i`. For
    /// call sites where the channel is a runtime variable rather than a `PROP_*` constant.
    #[inline]
    pub fn get(&self, i: usize, ch: usize) -> f32 {
        match ch {
            0 => self.wetness[i],
            1 => self.threshold[i],
            2 => self.flow_rate[i],
            3 => self.grain_size[i],
            _ => panic!("CellProps channel out of range: {ch}"),
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, ch: usize, v: f32) {
        match ch {
            0 => self.wetness[i] = v,
            1 => self.threshold[i] = v,
            2 => self.flow_rate[i] = v,
            3 => self.grain_size[i] = v,
            _ => panic!("CellProps channel out of range: {ch}"),
        }
    }

    /// Copies all four channels of `src` onto `dst`, in the same wetness/threshold/flow_rate/
    /// grain_size order the old `for ch in 0..4 { props[dst*4+ch] = props[src*4+ch] }` used.
    #[inline]
    pub fn copy_cell(&mut self, dst: usize, src: usize) {
        self.wetness[dst] = self.wetness[src];
        self.threshold[dst] = self.threshold[src];
        self.flow_rate[dst] = self.flow_rate[src];
        self.grain_size[dst] = self.grain_size[src];
    }

    #[inline]
    pub fn swap_cell(&mut self, a: usize, b: usize) {
        self.wetness.swap(a, b);
        self.threshold.swap(a, b);
        self.flow_rate.swap(a, b);
        self.grain_size.swap(a, b);
    }

    /// The historical interleaved `[wetness, threshold, flow_rate, grain_size] * len` layout,
    /// for the JS-facing boundary and any diagnostic that hashes/dumps that exact byte order.
    pub fn to_interleaved(&self) -> Vec<f32> {
        let n = self.len();
        let mut out = vec![0.0f32; n * 4];
        for i in 0..n {
            out[i * 4] = self.wetness[i];
            out[i * 4 + 1] = self.threshold[i];
            out[i * 4 + 2] = self.flow_rate[i];
            out[i * 4 + 3] = self.grain_size[i];
        }
        out
    }

    /// Copies from the historical interleaved layout, reproducing the old
    /// `self.cell_props[..len].copy_from_slice(&data[..len])` boundary semantics exactly --
    /// including a partial final cell when `data.len()` is not a multiple of 4.
    pub fn copy_from_interleaved(&mut self, data: &[f32]) {
        let total_len = self.len() * 4;
        let len = total_len.min(data.len());
        for flat in 0..len {
            self.set(flat / 4, flat % 4, data[flat]);
        }
    }

    pub fn from_interleaved(data: &[f32]) -> Self {
        let mut out = Self::new(data.len() / 4);
        out.copy_from_interleaved(data);
        out
    }
}

/// Packs one cell's RGBA bytes into a single `u32`, little-endian byte order `[r, g, b, a]` --
/// i.e. `bytemuck::cast_slice::<u32, u8>` on a whole `Vec<u32>` of these reproduces exactly the
/// historical RGBA-interleaved `Vec<u8>` layout on the little-endian targets this project ships
/// to (wasm32 and x86_64).
#[inline]
pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

#[inline]
pub fn unpack_rgba(c: u32) -> (u8, u8, u8, u8) {
    (c as u8, (c >> 8) as u8, (c >> 16) as u8, (c >> 24) as u8)
}

/// Read one byte channel (0=r, 1=g, 2=b, 3=a) out of a packed cell color.
#[inline]
pub fn color_channel(c: u32, ch: usize) -> u8 {
    (c >> (ch * 8)) as u8
}

/// Returns `c` with byte channel `ch` (0=r, 1=g, 2=b, 3=a) replaced by `v`, other channels
/// untouched -- a read-modify-write for call sites that used to write one interleaved byte at a
/// time.
#[inline]
pub fn set_color_channel(c: u32, ch: usize, v: u8) -> u32 {
    let shift = ch * 8;
    (c & !(0xFFu32 << shift)) | ((v as u32) << shift)
}

/// Overwrites packed colors from the historical RGBA-interleaved `u8` layout, reproducing the
/// old `self.cell_colors[..len].copy_from_slice(&data[..len])` boundary semantics exactly --
/// including a partial final cell when `data.len()` is not a multiple of 4.
pub fn colors_from_interleaved(dst: &mut [u32], data: &[u8]) {
    let total_len = dst.len() * 4;
    let len = total_len.min(data.len());
    for flat in 0..len {
        let i = flat / 4;
        let ch = flat % 4;
        dst[i] = set_color_channel(dst[i], ch, data[flat]);
    }
}

/// The historical RGBA-interleaved `u8` layout, for diagnostics/dumpers that hash or persist
/// that exact byte order.
pub fn colors_to_interleaved(src: &[u32]) -> Vec<u8> {
    let mut out = vec![0u8; src.len() * 4];
    for (i, &c) in src.iter().enumerate() {
        let (r, g, b, a) = unpack_rgba(c);
        out[i * 4] = r;
        out[i * 4 + 1] = g;
        out[i * 4 + 2] = b;
        out[i * 4 + 3] = a;
    }
    out
}

/// How often (in ticks) the quantile-line overlay pays a full `O(width*height)` row-mass
/// recompute, independent of `active_blocks`. See the call site in `update` for why this is
/// necessary in addition to the cheap every-5-tick `refresh_quantiles_partial` path: that path
/// only re-sums a row when some block in its block-row is active *in the exact tick it runs on*
/// (a single-tick snapshot, not an OR across skipped ticks), so a row a block touched and then
/// went permanently INACTIVE on an unsampled tick is never revisited and can hold a stale,
/// possibly nonzero, cached mass indefinitely. Bounds the staleness of any quantile line to at
/// most this many ticks. 100 is cheap amortised (one full grid sum per hundred single-tick
/// solver steps) against how rarely it needs to fire.
const QUANTILE_FULL_RESYNC_TICKS: u32 = 100;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SimulatorMode {
    Sandbox,
    SandFall,
}

impl Default for SimulatorMode {
    fn default() -> Self {
        Self::Sandbox
    }
}

// Explicit discriminants: the web UI and `sandart-wasm::set_sandbox_shape` map an independent,
// stable integer id to each variant (see that function's `match`), but `self.sandbox_shape as
// u32` (used for the render() shape uniform) casts the ENUM's own discriminant, which without
// explicit values is just declaration order. `MultiStageHourglass` (id 4) was removed from
// between `Hourglass` and `GaltonBoard` on 2026-09-19; pinning every remaining variant's
// discriminant to its pre-removal value keeps `as u32` in permanent agreement with the UI's ids
// -- including for GaltonBoard/StaircaseCascade/ProceduralFunnel/MultiNeckHourglass/
// UTubeFlowThrough, which would otherwise have silently shifted down by one. Id 4 is now unused.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxShape {
    Circle = 0,
    Square = 1,
    Oval = 2,
    Hourglass = 3,
    GaltonBoard = 5,
    StaircaseCascade = 6,
    ProceduralFunnel = 7,
    MultiNeckHourglass = 8,
    /// Task #61: a U-shaped flow-through vessel -- a test apparatus for pressure work, not a
    /// sand cascade. Water fills the tall left reservoir arm, flows down through a partly
    /// ROOFED bottom basin (deliberately: this is the Pascal-pressure test case), climbs the
    /// shorter right arm, spills over its rim (the overflow lip) through a horizontal spout,
    /// and falls into a catch well. See `physics::U_TUBE_RECTS` for the geometry.
    UTubeFlowThrough = 9,
    /// 12 chambers in a 4-column x 3-row grid, each chamber (rows 0-1) with two outlet pipes
    /// feeding chambers in the row below, plus a wide collector pool below row 2 -- the
    /// "network of chambers" the user approved from the Round-3 G4-family prototype
    /// (`sandart-sim/examples/proto_networks.rs`, `artifacts/design/network-2026-09-19/README.md`).
    /// The top row starts full and is the reservoir. Which two columns each chamber feeds is
    /// `network_routing` (R1/R2/R5); see `physics::eval_sandbox_shape_at`'s match arm and
    /// `NetworkRouting` for the routing tables. Id 10 -- id 4 (`MultiStageHourglass`, deleted
    /// 2026-09-19) stays retired, not reused.
    ChamberNetwork = 10,
}

impl Default for SandboxShape {
    fn default() -> Self {
        Self::Circle
    }
}

/// Which two columns of the row below each `ChamberNetwork` chamber feeds -- the only thing
/// that varies between the three shipped routings; see the Round-3 prototype
/// (`artifacts/design/network-2026-09-19/README.md`, "R1-R6") for the full family. Only R1, R2
/// and R5 shipped: R4 and R6 need a perfectly flat (0.089-repose-floor, i.e. zero-degree)
/// lateral connector purely for reachability, and that flat connector measurably strands
/// ~10% of the sand (9.8%/10.6% residual vs. R1/R2/R5's 0.4%/0.6%/0.9%). R3 is included in
/// neither the "ship" nor the "explicitly rejected" list in the prototype write-up but was not
/// asked for either, so it is left out too.
///
/// 2026-09-23: all three tables were redesigned so every pipe spans at most one column
/// (`|Δcol| <= 1`) -- see `physics::NetworkRoute`'s doc comment for why (thin pipes everywhere,
/// including what used to be R2/R5's multi-column "dogleg" pipes, cost more dry-sand drainage at
/// the elbow than the width bought back, so the wraparound/butterfly topology that needed those
/// long pipes was replaced instead of re-widened). The original prototype's R2 (a wrap from
/// column 3 back to column 0) and R5 (a stride-2 "butterfly") are gone; what ships as R2 and R5
/// now are this crate's own single-column-span designs, not the prototype's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkRouting {
    /// `[(0,1),(1,2),(2,3),(3,2)]` for both row transitions -- the baseline the user liked
    /// (originally "G4"), and the best of the six on both objective measures (water-mixing
    /// deviation and dry-sand residual). No crossing: each column mostly feeds itself and its
    /// right neighbour.
    R1,
    /// `[(0,1),(0,2),(2,3),(3,2)]` for both row transitions -- differs from R1 at column 1 only
    /// (columns 0 and 3 only have one valid neighbour each under the `|Δcol| <= 1` rule, so they
    /// match R1, and column 2 is also left as R1's chain). Column 1 skips itself and feeds column
    /// 0 AND column 2 instead, crossing over column 2's own stream.
    R2,
    /// `[(0,1),(0,2),(1,3),(3,2)]` top->middle -- the fuller "X" cross, both columns 1 and 2
    /// skipping themselves, not just column 1 like R2 -- then R1's chain table `[(0,1),(1,2),
    /// (2,3),(3,2)]` middle->bottom. Streams that crossed in the first transition recombine
    /// differently in the second, so a chamber's contents take a different-shaped path each row
    /// instead of the same table applied twice (as R1 and R2 both do).
    R5,
}

impl Default for NetworkRouting {
    fn default() -> Self {
        Self::R1
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MaterialMode {
    DrySand,
    KineticSand,
    WetSand,
    CoarseSand,
    ButterCream,
    Snow,
    FinePowder,
    MoonDust,
    Water,
    Milk,
    VegetableOil,
    CalmWater,
    Yogurt,
}

impl Default for MaterialMode {
    fn default() -> Self {
        Self::DrySand
    }
}

impl MaterialMode {
    /// Every material, in menu order. Single source of truth for anything that needs to
    /// enumerate materials (e.g. populating a UI select) — iterate this rather than hand-writing
    /// a parallel list, so there is nothing to fall out of sync.
    pub const ALL: [MaterialMode; 13] = [
        MaterialMode::DrySand,
        MaterialMode::KineticSand,
        MaterialMode::WetSand,
        MaterialMode::CoarseSand,
        MaterialMode::ButterCream,
        MaterialMode::Snow,
        MaterialMode::FinePowder,
        MaterialMode::MoonDust,
        MaterialMode::Water,
        MaterialMode::Milk,
        MaterialMode::VegetableOil,
        MaterialMode::CalmWater,
        MaterialMode::Yogurt,
    ];

    /// Stable string id. This — not the enum's numeric discriminant, and not array position —
    /// is the identity that should ever cross a language/process boundary (wasm, JSON, URLs).
    /// It only ever grows; never repurpose or remove an existing id, even for a deleted
    /// material, since old links/saved configs may still reference it.
    pub fn as_str(&self) -> &'static str {
        match self {
            MaterialMode::DrySand => "dry_sand",
            MaterialMode::KineticSand => "kinetic_sand",
            MaterialMode::WetSand => "wet_sand",
            MaterialMode::CoarseSand => "coarse_sand",
            MaterialMode::ButterCream => "butter_cream",
            MaterialMode::Snow => "snow",
            MaterialMode::FinePowder => "fine_powder",
            MaterialMode::MoonDust => "moon_dust",
            MaterialMode::Water => "water",
            MaterialMode::Milk => "milk",
            MaterialMode::VegetableOil => "vegetable_oil",
            MaterialMode::CalmWater => "calm_water",
            MaterialMode::Yogurt => "yogurt",
        }
    }

    /// Parse a stable string id back into a `MaterialMode`. Returns `None` for anything
    /// unrecognized rather than silently falling back to a default — callers at a language
    /// boundary (e.g. `sandart-wasm`) should surface that as an error, not eat it.
    pub fn from_str(s: &str) -> Option<MaterialMode> {
        Some(match s {
            "dry_sand" => MaterialMode::DrySand,
            "kinetic_sand" => MaterialMode::KineticSand,
            "wet_sand" => MaterialMode::WetSand,
            "coarse_sand" => MaterialMode::CoarseSand,
            "butter_cream" => MaterialMode::ButterCream,
            "snow" => MaterialMode::Snow,
            "fine_powder" => MaterialMode::FinePowder,
            "moon_dust" => MaterialMode::MoonDust,
            "water" => MaterialMode::Water,
            "milk" => MaterialMode::Milk,
            "vegetable_oil" => MaterialMode::VegetableOil,
            "calm_water" => MaterialMode::CalmWater,
            "yogurt" => MaterialMode::Yogurt,
            _ => return None,
        })
    }

    /// Human-readable display label for UI menus.
    pub fn label(&self) -> &'static str {
        match self {
            MaterialMode::DrySand => "Dry sand",
            MaterialMode::KineticSand => "Kinetic sand",
            MaterialMode::WetSand => "Wet sand",
            MaterialMode::CoarseSand => "Coarse sand",
            MaterialMode::ButterCream => "Buttercream",
            MaterialMode::Snow => "Snow",
            MaterialMode::FinePowder => "Fine powder",
            MaterialMode::MoonDust => "Moon dust",
            MaterialMode::Water => "Water",
            MaterialMode::Milk => "Milk",
            MaterialMode::VegetableOil => "Vegetable oil",
            MaterialMode::CalmWater => "Calm water",
            MaterialMode::Yogurt => "Yogurt",
        }
    }

    /// (wetness, threshold, flow_rate, grain_size) physics preset values. The single source of
    /// truth for these constants — `apply_preset` just writes them into `cell_props`, and
    /// external consumers that need them (e.g. the web UI's material-blend preview) should call
    /// this instead of keeping their own copy.
    pub fn preset_props(&self) -> (f32, f32, f32, f32) {
        match self {
            MaterialMode::DrySand => (0.00, 0.08, 0.25, 0.45),
            MaterialMode::CoarseSand => (0.00, 0.11, 0.22, 0.80),
            MaterialMode::KineticSand => (0.20, 0.10, 0.15, 0.35),
            MaterialMode::WetSand => (0.45, 0.14, 0.08, 0.40),
            MaterialMode::FinePowder => (0.00, 0.05, 0.30, 0.05),
            MaterialMode::Snow => (0.05, 0.15, 0.20, 0.20),
            MaterialMode::MoonDust => (0.00, 0.20, 0.20, 0.10),
            MaterialMode::ButterCream => (0.70, 0.04, 0.15, 0.08),
            MaterialMode::Water => (1.00, 0.00, 0.00, 0.00),
            MaterialMode::CalmWater => (0.90, 0.00, 0.00, 0.00),
            MaterialMode::Milk => (0.95, 0.00, 0.00, 0.00),
            MaterialMode::VegetableOil => (0.85, 0.00, 0.00, 0.00),
            MaterialMode::Yogurt => (0.75, 0.00, 0.00, 0.08),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockActivity {
    Inactive = 0,
    Slow = 1,
    Medium = 2,
    Fast = 3,
}

impl Default for BlockActivity {
    fn default() -> Self {
        Self::Inactive
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarbleState {
    pub pos: Vec2,
    pub prev_pos: Vec2,
    pub vel: Vec2,
    pub was_active: bool,
}

impl Default for MarbleState {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            prev_pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            was_active: false,
        }
    }
}

pub trait HeightmapSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]);
    fn reset(&mut self);
    fn heightmap(&self) -> &[f32];
    fn dimensions(&self) -> (usize, usize);
    fn marbles(&self) -> &[MarbleState; 5];
    fn active_bounds(&self) -> ActiveBounds;
}

/// Coordinates the state of the marble and the sand bed heightmap.
pub struct DrawingSimulation {
    /// The sand heightmap grid.
    pub heightmap: Heightmap,
    /// Pre-allocated temp buffer for double-buffering settling flows.
    pub temp_heights: Vec<f32>,
    /// Per-cell RGBA color buffer, RGBA interleaved. This is the simulation's single source of
    /// truth for color and is also exactly what external consumers (sandart-wasm's GPU upload,
    /// the native renderer, `set_cell_colors`'s `&[u8]` contract) read — there is no separate
    /// render view and no conversion step.
    ///
    /// `physics::advect_properties` blends in f32 internally and rounds back to `u8`
    /// *stochastically*, which is what keeps sub-LSB increments from being systematically
    /// discarded; see `physics::stochastic_round`.
    pub cell_colors: Vec<u32>,
    /// Per-cell physics & render properties. Advected with height. Structure-of-arrays: see
    /// `CellProps`.
    pub cell_props: CellProps,
    /// Current position of the primary marble (backward compatibility).
    pub marble_pos: Vec2,
    /// Previous position of the primary marble (backward compatibility).
    pub prev_marble_pos: Vec2,
    /// Last velocity of the primary marble (backward compatibility).
    pub marble_vel: Vec2,
    /// Track whether the primary marble has an active drawing stroke (backward compatibility).
    pub was_active: bool,
    /// Up to 5 marbles tracked in the simulation
    pub marbles: [MarbleState; 5],
    /// Active bounding box for settling updates.
    pub active_bounds: ActiveBounds,
    /// Sliding state tracker for stick-slip shear hysteresis.
    pub sliding: Vec<bool>,
    /// Per-edge momentum for the conservative edge-flux liquid solver (see
    /// `physics::flux_edge`). `edge_vel_h[i]` is the horizontal edge between cell `i` and
    /// `i + 1`; `edge_vel_v[i]` the vertical edge between cell `i` and `i + GRID_SIZE`.
    /// Replaces the old per-cell `wave_vel`, which could not be made mass-conservative.
    pub edge_vel_h: Vec<f32>,
    pub edge_vel_v: Vec<f32>,
    /// Depth-integrated lateral pressure bookkeeping for the cross-gravity liquid edge (see
    /// `physics::LATERAL_PRESSURE_SCALE` and the `column_depth` note in `settle_tick`). Persists
    /// tick-to-tick like `edge_vel_h`/`edge_vel_v` so a column under a sleeping block keeps the
    /// last depth it actually computed.
    pub column_depth: Vec<f32>,
    /// Seed for marble movement noise.
    pub seed: u32,

    // Internal simulation configuration fields
    pub marble_radius: f32,
    pub material_mode: MaterialMode,
    pub sandbox_shape: SandboxShape,
    pub gravity_dir: Vec2,
    pub neck_width: f32,
    pub hourglass_curve: f32,
    /// Which two columns of the row below each `SandboxShape::ChamberNetwork` chamber feeds.
    /// Unused by every other shape (same as `neck_width` is unused by `UTubeFlowThrough`).
    pub network_routing: NetworkRouting,

    /// Precomputed shape mask grid (GRID_SIZE * GRID_SIZE).
    /// Values: MASK_OUTSIDE (0) = wall, MASK_INSIDE (1) = playable interior,
    /// MASK_BOUNDARY (2) = inside but adjacent to a wall cell.
    pub shape_mask: Vec<u8>,
    /// Set to true when the shape_mask has been regenerated and needs GPU re-upload.
    pub shape_mask_dirty: bool,
    /// Whether the apparatus is currently upside down. Consumed by `generate_shape_mask` (it
    /// negates `dy` in the shape evaluator), so the *structure* inverts along with its contents
    /// — asymmetric shapes like StaircaseCascade used to keep their original orientation while
    /// the sand mirrored into them. Stored rather than applied to the
    /// mask in place because the mask is rebuilt from scratch whenever neck width, curvature or
    /// the shape itself changes, which would silently discard an in-place mirror.
    pub flipped: bool,







    /// STICKINESS.md: strength of the per-cell downward-flow jitter applied to UNDERFULL liquid,
    /// `0.0..=1.0`. `0.0` (the default) is bit-identical to before the feature existed.
    ///
    /// The user's ask was "reduce stickiness ... by making falling liquid a little more
    /// stochastic", and their choice of quantity: per-cell downward flow, gated on the cell being
    /// underfull. A cell at capacity is part of a column and is left alone -- jittering settled
    /// liquid would produce churn at rest, which is a regression this project already watches.
    /// A nearly-empty cell is the leading edge of a fall, where a perfectly uniform front is what
    /// reads as synthetic. See `physics::fall_flow_jitter` for the multiplier and why it only
    /// ever reduces.
    pub liquid_fall_jitter: f32,
















    /// Coarse block activity grid for CA optimization.
    pub active_blocks: Vec<BlockActivity>,
    /// Max displacement observed in each block during the last time it was simulated.
    pub last_displacements: Vec<f32>,
    /// Tick count of when each block was last simulated.
    pub last_simulated_ticks: Vec<u32>,
    /// Current dynamic simulation budget (N blocks).
    pub budget_n: usize,
    /// Exponential moving average of step time in milliseconds.
    pub ema_frame_ms: f32,
    /// Block size (e.g. 32 pixels).
    pub block_size: usize,
    /// Tick count for multi-rate LOD scheduling.
    pub tick_count: u32,

    /// Mass-weighted "how much has fallen" overlay setting (Sand-fall mode only). Off by
    /// default; setting this to anything other than `Off` is what turns on the per-row mass
    /// bookkeeping below — see `set_quantile_mode`.
    pub quantile_mode: QuantileMode,
    /// Cached per-*row* (not per-block) mass sum, `heightmap.height` entries long. Refreshed by
    /// `refresh_row_mass_active`/`refresh_row_mass_full`; only touched at all while
    /// `quantile_mode != QuantileMode::Off`, so it costs nothing when the feature is off.
    pub row_mass: Vec<f32>,
    /// The current quantile line targets (normalised 0.0..1.0, 0.0 = top row edge, 1.0 = bottom
    /// row edge), recomputed alongside `row_mass`. Length is 0 (Off), 3 (Quartiles), or 9
    /// (Deciles). These are raw targets, not eased for display — frame-to-frame smoothing is a
    /// rendering concern and belongs to the consumer (sandart-wasm), not the simulation.
    quantile_targets: Vec<f32>,


    /// How many times the cross-gravity (lateral) edge pass runs per tick, as a real-valued dial
    /// (NOT an integer count) -- the fix for a settled liquid facet staying at a straight ~45
    /// degrees instead of flattening, since both the lateral and vertical edge solvers move at
    /// most one cell of fill per tick and a full donor cell has no room to pass mass through it
    /// sideways at the rate gravity keeps stacking it. `1.0` (the default here) is BIT-IDENTICAL
    /// to before this field existed, and so is any integer value. A fractional dial's fractional
    /// part is realised STOCHASTICALLY, once per tick, globally (never per block or per cell): the
    /// tick runs `floor(lateral_substeps)` extra lateral passes, plus one more with probability
    /// `fract(lateral_substeps)`, so expected pass count across many ticks equals the dial value
    /// exactly, and a fully wet donor only ever pays for whole passes -- never a partial last one.
    /// DRY material is completely unaffected at every value (the extra weight is a continuous
    /// function of wetness with no liquid-only or sand-only gate -- see `physics::settle_tick`'s
    /// `lateral_substeps` parameter doc comment for the full mechanism, the stochastic realisation,
    /// and the traps both have to avoid). Not yet exposed to the UI beyond `set_lateral_substeps`
    /// in `sandart-wasm`; the shipped default is chosen after measuring, not assumed here.
    pub lateral_substeps: f32,

}

fn generate_smooth_noise(seed_val: u32, grid_size: usize) -> Heightmap {
    let mut heightmap = Heightmap::new(grid_size, grid_size, DEFAULT_SAND_HEIGHT);
    let mut seed = seed_val;

    // Helper to generate a low-res random grid via XORShift
    let mut gen_grid = |size: usize| -> Vec<f32> {
        let mut grid = vec![0.0f32; size * size];
        for val in grid.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *val = (seed as f32 / u32::MAX as f32) - 0.5; // Range [-0.5, 0.5]
        }
        grid
    };

    // Generate two noise grids at different resolutions (octaves)
    let grid_size1 = 8;
    let grid1 = gen_grid(grid_size1);

    let grid_size2 = 16;
    let grid2 = gen_grid(grid_size2);

    // Bilinear interpolation helper with smoothstep
    let sample_octave = |grid: &[f32], size: usize, x: usize, y: usize| -> f32 {
        let fx = (x as f32 / (grid_size - 1) as f32) * (size - 1) as f32;
        let fy = (y as f32 / (grid_size - 1) as f32) * (size - 1) as f32;

        let x0 = fx.floor() as usize;
        let x1 = (x0 + 1).min(size - 1);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(size - 1);

        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;

        // Smoothstep interpolation
        let sx = tx * tx * (3.0 - 2.0 * tx);
        let sy = ty * ty * (3.0 - 2.0 * ty);

        let v00 = grid[y0 * size + x0];
        let v10 = grid[y0 * size + x1];
        let v01 = grid[y1 * size + x0];
        let v11 = grid[y1 * size + x1];

        let h0 = v00 * (1.0 - sx) + v10 * sx;
        let h1 = v01 * (1.0 - sx) + v11 * sx;
        h0 * (1.0 - sy) + h1 * sy
    };

    for y in 0..grid_size {
        let row_offset = y * grid_size;
        for x in 0..grid_size {
            // Combine octaves: 8x8 primary (amp 0.025), 16x16 secondary (amp 0.008)
            let val1 = sample_octave(&grid1, grid_size1, x, y) * 0.025;
            let val2 = sample_octave(&grid2, grid_size2, x, y) * 0.008;

            let combined = val1 + val2;
            heightmap.data[row_offset + x] = (DEFAULT_SAND_HEIGHT + combined).clamp(0.0, 1.0);
        }
    }

    heightmap
}

impl DrawingSimulation {
    pub fn new() -> Self {
        Self::new_with_size(GRID_SIZE)
    }

    /// Construct a simulation over a `grid_size` x `grid_size` grid. `GRID_SIZE` (512) is the
    /// shipped default (`new()` calls this with `GRID_SIZE`); the web UI additionally offers
    /// 64/128/256 as a debugging/perf instrument — see `docs/ARCHITECTURE.md` and the
    /// resolution-selector plumbing in `sandart-wasm`.
    ///
    /// `block_size` (the LOD scheduler's block edge length in cells) is a CONSTANT 8 cells at
    /// every shipped resolution — see `DEFAULT_BLOCK_SIZE`. The block grid therefore scales with
    /// the grid: 8x8 at 64, 16x16 at 128, 32x32 at 256, 64x64 at 512.
    ///
    /// **This reverses the `grid_size / 64` geometry** that pinned the block count at 64x64 = 4096
    /// for every resolution. That was adopted so the LOD block and `coarse::CoarseGeometry`'s
    /// pressure tile were the same square (`COARSE_GRID` was also 64); the coarse level was
    /// deleted on 2026-08-30, so the reason is gone while the cost was not — `block_size` was 1 at
    /// grid 64 (one block per cell, the LOD scheduler degenerate) and 2 at grid 128 (the slab
    /// artifact `VERTICAL_PRESSURE_CAP_MULT` documents). At grid 512 the two geometries are
    /// identical, so this is a no-op at the shipped default.
    ///
    /// Because the block count is resolution-dependent again, `budget_n` and the adaptive
    /// controller's throttles are now DERIVED from it (`budget_throttles`) rather than hardcoded,
    /// so they stay the same fraction of the block grid at every resolution — which is the
    /// property the `/64` geometry was also trying to buy, obtained directly instead.
    ///
    /// **If you re-wire the block heat-map overlay, read this.** `sandart-render`'s
    /// `update_block_heat` uploads into a texture of fixed `HEAT_GRID_SIZE` (64) square with no
    /// bounds check on the source slice, so it is only safe while the block grid is exactly 64x64
    /// — true at grid 512, false at 64/128/256 under this geometry. That path is currently DEAD
    /// (`update_block_heat` has no callers and `block_heat_texels` was deleted with the overlays),
    /// which is why the constraint no longer binds. Reviving it means sizing the texture from the
    /// block grid, not from a constant.
    ///
    pub fn new_with_size(grid_size: usize) -> Self {
        Self::new_with_block_divisor(grid_size, default_block_divisor(grid_size))
    }

    /// `new_with_size`, with the LOD block edge length left open: `block_size = grid/divisor`.
    /// The shipped geometry is `default_block_divisor(grid)`, i.e. a constant 8-cell block; a
    /// SMALLER divisor means BIGGER blocks (16 -> 32-cell blocks at grid 512). Exists so block
    /// size can be MEASURED rather than argued about -- see BLOCK-SIZE-SWEEP.md.
    ///
    /// `block_size` is floored at 1, so a divisor larger than `grid_size` silently clamps.
    pub fn new_with_block_divisor(grid_size: usize, divisor: usize) -> Self {
        let heightmap = generate_smooth_noise(12345u32, grid_size);
        let temp_heights = heightmap.data.clone();
        let sliding = vec![false; grid_size * grid_size];
        let edge_vel_h = vec![0.0f32; grid_size * grid_size];
        let edge_vel_v = vec![0.0f32; grid_size * grid_size];
        let column_depth = vec![0.0f32; grid_size * grid_size];
        let cell_colors = vec![pack_rgba(210, 180, 140, 255); grid_size * grid_size];
        // Initialize with default DrySand preset
        let cell_props = CellProps::filled(grid_size * grid_size, 0.00, 0.08, 0.25, 0.45);

        // See the doc comment above: this scales with grid_size so the block-count (and
        // therefore the meaning of budget_n, and the heat-map overlay's fixed-size texture
        // upload) stays resolution-invariant. Floor stays `.max(1)`, unchanged from before this
        // change -- see the doc comment for why grid 64's resulting block_size=1 is accepted
        // rather than floored to 2.
        let block_size = (grid_size / divisor.max(1)).max(1);
        let cols = (grid_size + block_size - 1) / block_size;
        let rows = (grid_size + block_size - 1) / block_size;
        let active_blocks = vec![BlockActivity::Inactive; cols * rows];
        let last_displacements = vec![0.0f32; cols * rows];
        let last_simulated_ticks = vec![0u32; cols * rows];
        // A quarter of the block grid -- 1024 at grid 512, the value this was hardcoded to while
        // the block count was pinned at 4096. See `budget_throttles`, `reset()` below for the
        // other site, and the throttle in `update`.
        let budget_n = budget_throttles(cols * rows).0;
        let ema_frame_ms = 33.3;

        let mut sim = Self {
            heightmap,
            temp_heights,
            cell_colors,
            cell_props,
            marble_pos: Vec2::ZERO,
            prev_marble_pos: Vec2::ZERO,
            marble_vel: Vec2::ZERO,
            was_active: false,
            marbles: [MarbleState::default(); 5],
            active_bounds: ActiveBounds {
                min_x: 0,
                max_x: 0,
                min_y: 0,
                max_y: 0,
                active: false,
            },
            sliding,
            edge_vel_h,
            edge_vel_v,
            column_depth,
            seed: 98765u32,
            marble_radius: 0.018,
            material_mode: MaterialMode::default(),
            sandbox_shape: SandboxShape::default(),
            gravity_dir: Vec2::ZERO,
            neck_width: 0.005,
            hourglass_curve: 0.6,
            network_routing: NetworkRouting::default(),
            shape_mask: vec![MASK_OUTSIDE; grid_size * grid_size],
            shape_mask_dirty: true,
            flipped: false,
            // Placeholder until `generate_shape_mask()` below does the real build -- shape_mask
            // is still all-MASK_OUTSIDE at this point in construction, so there is nothing
            // meaningful to build from yet.
            // Defaults ON -- see this field's own doc comment for why it differs from every
            // other debug toggle in the group, which default off.
            // Defaults OFF -- see this field's own doc comment for why (OVERCLOCKING.md split
            // it from the coarse level's own dynamics, which now run unconditionally).
            liquid_fall_jitter: 0.0,
            // LATERAL-COARSE-CORRECTION.md. Default OFF, like every other debug toggle in this
            // group. `COARSE_CORRECTION_DEFAULT_DAMPING` is a starting value, not a measured
            // optimum -- see its own doc comment.
            // CREDIT-DEBT-TRANSPORT.md §2.3. Default OFF, like every other debug toggle in this
            // group, and untested -- nothing about it has been measured yet.
            active_blocks,
            last_displacements,
            last_simulated_ticks,
            budget_n,
            ema_frame_ms,
            block_size,
            tick_count: 0,
            quantile_mode: QuantileMode::default(),
            row_mass: vec![0.0f32; grid_size],
            quantile_targets: Vec::new(),
            lateral_substeps: 1.0,
        };
        sim.generate_shape_mask();
        sim
    }

    /// Regenerate the shape mask from the current sandbox_shape, neck_width, hourglass_curve.
    /// Call this whenever these parameters change. Sets shape_mask_dirty for GPU re-upload.
    ///
    /// The `out_size == sim size` case of `rasterize_shape_mask` below -- kept as its own method
    /// (rather than a one-line call site) since `self.shape_mask`/`shape_mask_dirty` are the sim's
    /// own fields, not something a generic rasteriser should reach into.
    pub fn generate_shape_mask(&mut self) {
        let w = self.heightmap.width;
        debug_assert_eq!(w, self.heightmap.height, "sandbox grid is always square");
        self.shape_mask = self.rasterize_shape_mask(w);
        self.shape_mask_dirty = true;
    }

    /// Rasterize the CURRENT vessel shape (sandbox_shape, neck_width, hourglass_curve,
    /// flipped) at an arbitrary square output resolution `out_size`,
    /// returning a fresh `MASK_OUTSIDE`/`MASK_INSIDE`/`MASK_BOUNDARY` buffer -- the same values
    /// `shape_mask` holds, just not written into it.
    ///
    /// Always evaluates the geometry in SIM-cell coordinates (`w = h = S`, the actual simulation
    /// grid, i.e. `self.heightmap.width`), regardless of `out_size`: output pixel `(i, j)` samples
    /// the continuous sim-cell position `((i + 0.5) * S / out_size - 0.5, (j + 0.5) * S /
    /// out_size - 0.5)` via `physics::eval_sandbox_shape_at` -- the pixel-CENTRE convention that
    /// makes `out_size == S` sample EXACTLY the integer cell centres `eval_sandbox_shape` itself
    /// uses (`(i + 0.5) * S / S - 0.5 == i` bit-for-bit: `S / S` is exactly `1.0` for any nonzero
    /// float, and `i + 0.5 - 0.5` round-trips exactly for every representable `i` in this range).
    /// That is what makes `generate_shape_mask` above (`out_size = S`) bit-identical to the mask
    /// this crate produced before this function existed -- see
    /// `test_rasterize_shape_mask_matches_discrete_eval_at_sim_size`.
    ///
    /// This is the ONE place the vessel outline is rasterized at a resolution other than the sim
    /// grid (the render-resolution "fine mask" `sandart-wasm` uploads when downscaled, `out_size =
    /// render_size`) -- reusing `eval_sandbox_shape_at` rather than a second copy of the shape math
    /// is what keeps the render-resolution outline unable to drift from the physics: any new shape
    /// parameter added to `eval_sandbox_shape_at` reaches both masks through this one function.
    ///
    /// Pass 2 (boundary detection) runs at `out_size` too, so a `MASK_BOUNDARY` cell in the
    /// returned buffer means "adjacent to an OUTSIDE cell AT THIS RESOLUTION", not a resampling of
    /// the sim's own boundary cells.
    pub fn rasterize_shape_mask(&self, out_size: usize) -> Vec<u8> {
        let s = self.heightmap.width;
        debug_assert_eq!(s, self.heightmap.height, "sandbox grid is always square");
        let s_f = s as f32;
        let out_f = out_size as f32;

        let mut mask = vec![MASK_OUTSIDE; out_size * out_size];

        // Pass 1: evaluate inside/safe for every output pixel, sampled at its centre in sim-cell
        // coordinates, using the existing physics evaluator.
        for j in 0..out_size {
            let py = (j as f32 + 0.5) * s_f / out_f - 0.5;
            let offset = j * out_size;
            for i in 0..out_size {
                let px = (i as f32 + 0.5) * s_f / out_f - 0.5;
                let (inside, _safe) = physics::eval_sandbox_shape_at(
                    px, py, s, s,
                    self.sandbox_shape,
                    self.neck_width,
                    self.hourglass_curve,
                    self.flipped,
                    self.network_routing,
                );
                mask[offset + i] = if inside { MASK_INSIDE } else { MASK_OUTSIDE };
            }
        }

        // Pass 2: mark boundary cells - any INSIDE cell with at least one OUTSIDE neighbor, at
        // `out_size` resolution. We need a temporary copy to avoid read/write conflict.
        let snapshot = mask.clone();
        for y in 0..out_size {
            let offset = y * out_size;
            for x in 0..out_size {
                if snapshot[offset + x] == MASK_INSIDE {
                    let has_outside_neighbor =
                        (x == 0 || snapshot[offset + x - 1] == MASK_OUTSIDE) ||
                        (x + 1 >= out_size || snapshot[offset + x + 1] == MASK_OUTSIDE) ||
                        (y == 0 || snapshot[(y - 1) * out_size + x] == MASK_OUTSIDE) ||
                        (y + 1 >= out_size || snapshot[(y + 1) * out_size + x] == MASK_OUTSIDE);
                    if has_outside_neighbor {
                        mask[offset + x] = MASK_BOUNDARY;
                    }
                }
            }
        }

        mask
    }

    /// Return a pointer to the shape mask data for WASM/GPU access.
    pub fn shape_mask_ptr(&self) -> *const u8 {
        self.shape_mask.as_ptr()
    }

    /// Return the length of the shape mask data.
    pub fn shape_mask_len(&self) -> usize {
        self.shape_mask.len()
    }

    /// Reset the simulation state.
    pub fn reset(&mut self) {
        // A reset returns the apparatus to its upright orientation, so clear this before
        // rebuilding the mask rather than resetting into whatever way up it was left.
        self.flipped = false;
        self.generate_shape_mask();
        if matches!(
            self.sandbox_shape,
            SandboxShape::Hourglass
                | SandboxShape::GaltonBoard
                | SandboxShape::StaircaseCascade
                | SandboxShape::ProceduralFunnel
                | SandboxShape::MultiNeckHourglass
                | SandboxShape::UTubeFlowThrough
                | SandboxShape::ChamberNetwork
        ) {
            self.heightmap.reset(0.0);
            self.initialize_hourglass();
        } else {
            self.heightmap = generate_smooth_noise(54321u32, self.heightmap.width);
            self.temp_heights.copy_from_slice(&self.heightmap.data);
        }
        self.sliding.fill(false);
        self.edge_vel_h.fill(0.0);
        self.edge_vel_v.fill(0.0);
        self.column_depth.fill(0.0);
        // Deliberately does NOT touch `cell_colors`. A reset is a physics-state reset (heights,
        // velocities, bounds) — the color theme the caller pushed via `set_cell_colors` is a
        // separate concern and has no reason to revert to the placeholder tan `new_with_size`
        // seeds a brand-new sim with. This used to unconditionally overwrite every cell back to
        // that placeholder here, which silently discarded whatever color theme was active any
        // time `reset()` ran — including from `set_sandbox_shape` on every Hourglass-family shape
        // change, not just the explicit Reset button. Preserving the buffer by simply not writing
        // to it means every current and future caller of `reset()` gets this for free, rather than
        // needing to remember to re-push the theme afterward (which is exactly the bug: one such
        // call site — `set_sandbox_shape` — was missed).
        self.apply_preset(self.material_mode);
        self.marble_pos = Vec2::ZERO;
        self.prev_marble_pos = Vec2::ZERO;
        self.marble_vel = Vec2::ZERO;
        self.was_active = false;
        self.marbles = [MarbleState::default(); 5];
        self.active_bounds = ActiveBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            active: false,
        };
        self.seed = 98765u32;
        self.active_blocks.fill(BlockActivity::Inactive);
        self.last_displacements.fill(0.0);
        self.last_simulated_ticks.fill(0);
        // Keep in sync with `new_with_size`'s `budget_n` initialisation above.
        self.budget_n = budget_throttles(self.active_blocks.len()).0;
        self.ema_frame_ms = 33.3;
        self.tick_count = 0;
        self.refresh_quantiles_full();
    }

    pub fn initialize_hourglass(&mut self) {
        // Self-sufficient: regenerate the mask here rather than trusting callers to have
        // done so already for the current sandbox_shape/neck_width/hourglass_curve.
        self.generate_shape_mask();

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let center_x = w as f32 / 2.0;
        let center_y = h as f32 / 2.0;

        for y in 0..h {
            let row_offset = y * w;
            let dy = y as f32 - center_y;
            for x in 0..w {
                let idx = row_offset + x;
                let inside = self.shape_mask[idx] != MASK_OUTSIDE;

                // A single scalar `dy < fill_threshold` cutoff (the path every other shape
                // below takes) cannot express "fill only the reservoir arm": the right arm and
                // catch well overlap the reservoir's dy range, so any threshold that fills the
                // reservoir also fills them. Instead, fill exactly the reservoir rect --
                // `physics::U_TUBE_RECTS[U_TUBE_RESERVOIR_RECT]`, the same constant the mask
                // geometry itself is built from, so this can never drift out of sync with it.
                if self.sandbox_shape == SandboxShape::UTubeFlowThrough {
                    let dx = x as f32 - center_x;
                    let r = &physics::U_TUBE_RECTS[physics::U_TUBE_RESERVOIR_RECT];
                    let in_reservoir = dx >= r[0] * w as f32
                        && dx < r[1] * w as f32
                        && dy >= r[2] * h as f32
                        && dy < r[3] * h as f32;
                    self.heightmap.data[idx] = if inside && in_reservoir { 1.00 } else { 0.0 };
                    continue;
                }

                // ChamberNetwork's reservoir (row 0, the top row of chambers) is NOT the
                // `dy < 0.0` half every other shape below uses -- row 0 is enlarged to 40% of
                // the vertical budget (`physics::NET_ROW_FRACS`), so its lower edge sits well
                // above the vertical centreline. Using the generic `dy < 0.0` threshold here
                // would also fill roughly the top half of row 1's chambers, since row 1 straddles
                // dy = 0. `physics::chamber_network_reservoir_boundary` is the single source of
                // truth this and `eval_sandbox_shape_at`'s geometry share.
                if self.sandbox_shape == SandboxShape::ChamberNetwork {
                    let boundary = physics::chamber_network_reservoir_boundary(h as f32);
                    self.heightmap.data[idx] = if inside && dy < boundary { 1.00 } else { 0.0 };
                    continue;
                }

                let fill_threshold = if self.sandbox_shape == SandboxShape::StaircaseCascade {
                    -0.26 * h as f32
                } else {
                    0.0
                };

                if inside {
                    if dy < fill_threshold {
                        // Upper chamber: filled with smooth sand (1.00 height / 100% capacity)
                        self.heightmap.data[idx] = 1.00;
                    } else {
                        // Lower chamber / lower stages: empty
                        self.heightmap.data[idx] = 0.0;
                    }
                } else {
                    self.heightmap.data[idx] = 0.0;
                }
            }
        }
        self.temp_heights.copy_from_slice(&self.heightmap.data);
    }

    pub fn flip_hourglass(&mut self) {
        let w = self.heightmap.width;
        let h = self.heightmap.height;
        
        // Symmetrical reflection around center_y (h / 2) so row 32 (neck) stays fixed
        for y in 1..=h / 2 {
            let y2 = h.saturating_sub(y);
            if y == y2 || y2 >= h {
                continue;
            }
            for x in 0..w {
                let i1 = y * w + x;
                let i2 = y2 * w + x;
                self.heightmap.data.swap(i1, i2);
                self.temp_heights.swap(i1, i2);
                // (edge momentum is not mirrored here — it is cleared after the loop)
                self.sliding.swap(i1, i2);

                self.cell_colors.swap(i1, i2);
                self.cell_props.swap_cell(i1, i2);
            }
        }

        // Edge momentum does not survive turning the apparatus over. Mirroring it would mean
        // reversing the sign of every gravity-aligned edge and shifting its index by one row,
        // and the partial row range swapped above does not cover the edge set cleanly anyway.
        // Clearing is both simpler and the physically honest answer: the contents are in free
        // fall from rest the instant the glass is inverted.
        self.edge_vel_h.fill(0.0);
        self.edge_vel_v.fill(0.0);
        self.column_depth.fill(0.0);

        // Turn the *structure* over too, not just what is in it. Symmetric shapes are unaffected
        // by construction; the asymmetric ones (StaircaseCascade's alternating shelves,
        // ProceduralFunnel's noise) used to stay upright while their contents mirrored into them.
        //
        // This must run BEFORE the out-of-bounds cleanup below, or that loop culls the mirrored
        // sand against the *old* geometry and deletes mass that the new geometry has room for.
        self.flipped = !self.flipped;
        self.generate_shape_mask();

        // Clean up any sand outside the shape boundary so no specs stay trapped outside/above ceiling
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if self.shape_mask[idx] == MASK_OUTSIDE {
                    self.heightmap.data[idx] = 0.0;
                    self.temp_heights[idx] = 0.0;
                }
            }
        }

        self.active_blocks.fill(BlockActivity::Inactive);
        self.last_displacements.fill(0.5); // Force all blocks to be re-simulated
        self.tick_count = 0;
        self.refresh_quantiles_full();
    }

    /// Set the quantile-line overlay mode (off/quartiles/deciles) and immediately bring the
    /// row-mass cache and quantile targets up to date. A full recompute here (rather than
    /// waiting for the next periodic partial refresh) means turning the feature on never shows
    /// a stale/zero reading left over from whatever the cache last held while it was off.
    pub fn set_quantile_mode(&mut self, mode: QuantileMode) {
        self.quantile_mode = mode;
        self.refresh_quantiles_full();
    }

    /// Current quantile line targets, normalised 0.0 (top row edge) .. 1.0 (bottom row edge).
    /// Empty when `quantile_mode` is `Off`. These are raw targets refreshed at most every 5
    /// ticks — easing them for smooth frame-to-frame motion is left to the renderer/consumer.
    pub fn quantile_positions(&self) -> &[f32] {
        &self.quantile_targets
    }

    /// Full (all `GRID_SIZE` rows) row-mass recompute plus a fresh quantile target computation.
    /// O(GRID_SIZE^2); only meant for discontinuities (reset, flip, mode just switched on) — the
    /// steady-state per-tick path is `refresh_quantiles_partial`, which only re-sums rows whose
    /// block was actually simulated this tick. A no-op (aside from clearing the cached targets)
    /// when the feature is off, so resets/flips stay free of this cost in the common case.
    fn refresh_quantiles_full(&mut self) {
        if self.quantile_mode == QuantileMode::Off {
            self.quantile_targets.clear();
            return;
        }
        refresh_row_mass_full(
            &self.heightmap.data,
            self.heightmap.width,
            self.heightmap.height,
            &self.shape_mask,
            &mut self.row_mass,
        );
        self.quantile_targets =
            compute_quantile_positions(&self.row_mass, self.quantile_mode.fractions());
    }

    /// Steady-state per-tick refresh: only re-sums rows belonging to a block that
    /// `settle_tick` actually simulated this tick (per `active_blocks`), then recomputes the
    /// quantile targets from the (mostly-cached) row_mass array. Called at most once every 5
    /// ticks from `update`, and only while `quantile_mode != Off` — see the call site for the
    /// full cost-gating rationale.
    fn refresh_quantiles_partial(&mut self) {
        refresh_row_mass_active(
            &self.heightmap.data,
            self.heightmap.width,
            self.heightmap.height,
            &self.shape_mask,
            self.block_size,
            &self.active_blocks,
            &mut self.row_mass,
        );
        self.quantile_targets =
            compute_quantile_positions(&self.row_mass, self.quantile_mode.fractions());
    }

    /// Apply a preset to the per-cell properties buffer.
    pub fn apply_preset(&mut self, mode: MaterialMode) {
        let (wetness, threshold, flow_rate, grain_size) = mode.preset_props();
        self.cell_props.wetness.fill(wetness);
        self.cell_props.threshold.fill(threshold);
        self.cell_props.flow_rate.fill(flow_rate);
        self.cell_props.grain_size.fill(grain_size);
        self.material_mode = mode;
    }

    /// Copy color patterns into CPU color buffer. `rgba_data` stays RGBA-interleaved -- the
    /// JS-facing contract is unchanged; it is converted to the packed-`u32` storage at this
    /// boundary.
    pub fn set_cell_colors(&mut self, rgba_data: &[u8]) {
        colors_from_interleaved(&mut self.cell_colors, rgba_data);
    }

    /// Copy per-cell properties from a custom buffer. `props_data` stays
    /// [wetness, threshold, flow_rate, grain_size]-interleaved -- the JS-facing contract is
    /// unchanged; it is converted to the structure-of-arrays storage at this boundary.
    pub fn set_cell_props(&mut self, props_data: &[f32]) {
        self.cell_props.copy_from_interleaved(props_data);
    }

    /// Convert normalized Cartesian coordinates ([-1.0, 1.0]) to grid index coordinates.
    #[allow(dead_code)]
    pub fn norm_to_grid(pos: Vec2, width: usize, height: usize) -> (usize, usize) {
        let px = if pos.x.is_finite() { pos.x } else { 0.0 };
        let py = if pos.y.is_finite() { pos.y } else { 0.0 };
        let x = ((px + 1.0) * 0.5 * width as f32).clamp(0.0, (width - 1) as f32) as usize;
        let y = ((1.0 - py) * 0.5 * height as f32).clamp(0.0, (height - 1) as f32) as usize;
        (x, y)
    }

    /// Erase height values inside the marble radius to 0.0 with sub-pixel precision.
    #[allow(dead_code)]
    pub fn draw_point(&mut self, pos: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors,
            &mut self.cell_props,
            pos,
            pos,
            radius,
            &mut self.active_bounds,
        );
    }

    /// Draw a line between start and end using interpolation to prevent gaps.
    #[allow(dead_code)]
    pub fn draw_line(&mut self, start: Vec2, end: Vec2, radius: f32) {
        displace_line(
            &mut self.heightmap,
            &mut self.cell_colors,
            &mut self.cell_props,
            start,
            end,
            radius,
            &mut self.active_bounds,
        );
    }

    fn clamp_to_sandbox(pos: Vec2, shape: SandboxShape, marble_radius: f32) -> Vec2 {
        let max_r = (0.92 - marble_radius).max(0.0);
        match shape {
            SandboxShape::Circle => {
                let len = pos.length();
                if len > max_r && len > 1e-5 {
                    pos * (max_r / len)
                } else {
                    pos
                }
            }
            SandboxShape::Square => {
                Vec2::new(
                    pos.x.clamp(-max_r, max_r),
                    pos.y.clamp(-max_r, max_r),
                )
            }
            SandboxShape::Oval => {
                let a = (0.92 - marble_radius).max(0.01);
                let b = (0.60 - marble_radius).max(0.01);
                let d_sq = (pos.x * pos.x) / (a * a) + (pos.y * pos.y) / (b * b);
                if d_sq > 1.0 {
                    let d = d_sq.sqrt();
                    pos / d
                } else {
                    pos
                }
            }
            SandboxShape::Hourglass
            | SandboxShape::GaltonBoard
            | SandboxShape::StaircaseCascade
            | SandboxShape::ProceduralFunnel
            | SandboxShape::MultiNeckHourglass
            | SandboxShape::UTubeFlowThrough
            | SandboxShape::ChamberNetwork => {
                let chamber_r = 0.92 - marble_radius;  // normalized coords
                let chamber_offset = 0.58;             // normalized vertical offset
                let neck_hw = 0.07 - marble_radius;    // normalized neck half-width

                // Check if in upper chamber, lower chamber, or neck
                let in_upper = Vec2::new(pos.x, pos.y - chamber_offset).length() < chamber_r;
                let in_lower = Vec2::new(pos.x, pos.y + chamber_offset).length() < chamber_r;
                let in_neck = pos.x.abs() < neck_hw && pos.y.abs() < chamber_offset;

                if in_upper || in_lower || in_neck {
                    pos  // already inside
                } else {
                    // Clamp to nearest boundary (upper or lower chamber)
                    let to_upper = Vec2::new(pos.x, pos.y - chamber_offset);
                    let to_lower = Vec2::new(pos.x, pos.y + chamber_offset);
                    if to_upper.length() < to_lower.length() {
                        let dir = to_upper.normalize_or_zero();
                        Vec2::new(0.0, chamber_offset) + dir * chamber_r
                    } else {
                        let dir = to_lower.normalize_or_zero();
                        Vec2::new(0.0, -chamber_offset) + dir * chamber_r
                    }
                }
            }
        }
    }

    /// Run a physics frame tick.
    pub fn update(&mut self, dt: f32, targets: &[Option<Vec2>; 5], marble_radius: f32, _material: MaterialMode, shape: SandboxShape, last_frame_time_ms: f32, target_frame_time_ms: f32) {
        // Prevent seed degeneracy (XORShift stuck state at 0)
        if self.seed == 0 {
            self.seed = 98765u32;
        }

        // Advance seed every frame to keep settling dynamics active and non-deterministic
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let time_seed = self.seed;

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let block_size = self.block_size;
        let cols = (w + block_size - 1) / block_size;
        let rows = (h + block_size - 1) / block_size;

        for j in 0..5 {
            if let Some(target) = targets[j] {
                // Sanitize target coordinate float boundaries against NaNs/Infs
                let tx = if target.x.is_finite() { target.x } else { 0.0 };
                let ty = if target.y.is_finite() { target.y } else { 0.0 };
                let target_sanitized = Vec2::new(tx, ty);

                let clamped_target = Self::clamp_to_sandbox(target_sanitized, shape, marble_radius);

                let mut segment_bounds = ActiveBounds {
                    min_x: 0,
                    max_x: 0,
                    min_y: 0,
                    max_y: 0,
                    active: false,
                };

                if self.marbles[j].was_active {
                    self.marbles[j].prev_pos = self.marbles[j].pos;

                    // Calculate step vector and distance
                    let raw_diff = clamped_target - self.marbles[j].pos;
                    let raw_dist = raw_diff.length();

                    // 1. Generate pseudo-random numbers
                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n1 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    let n2 = (self.seed as f32 / u32::MAX as f32 - 0.5) * 2.0; // [-1.0, 1.0]

                    let random_offset = Vec2::new(n1, n2);

                    // 2. Micro-jitter: simulate bumping over discrete sand grains (extremely subtle)
                    let jitter_amplitude = marble_radius * 0.04;
                    let jitter = random_offset * jitter_amplitude;

                    // 3. Inertia/drag drift: simulate sand resistance lagging and sliding sideways
                    let mut drift = Vec2::ZERO;
                    if raw_dist > 1e-5 {
                        let dir = raw_diff / raw_dist;
                        let perp = Vec2::new(-dir.y, dir.x);

                        // Minor drag (lag behind magnet/target)
                        let lag = -dir * (raw_dist * 0.08);

                        // Minor sideways slip (uneven resistance)
                        let slip = perp * (raw_dist * 0.05 * n1);

                        drift = lag + slip;
                    }

                    let mut next_pos = clamped_target + jitter + drift;
                    next_pos = Self::clamp_to_sandbox(next_pos, shape, marble_radius);

                    self.marbles[j].pos = next_pos;
                    self.marbles[j].vel = next_pos - self.marbles[j].prev_pos;

                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors,
                        &mut self.cell_props,
                        self.marbles[j].prev_pos,
                        self.marbles[j].pos,
                        marble_radius,
                        &mut segment_bounds,
                    );
                } else {
                    self.marbles[j].pos = clamped_target;
                    self.marbles[j].prev_pos = clamped_target;
                    self.marbles[j].vel = Vec2::ZERO;
                    displace_line(
                        &mut self.heightmap,
                        &mut self.cell_colors,
                        &mut self.cell_props,
                        clamped_target,
                        clamped_target,
                        marble_radius,
                        &mut segment_bounds,
                    );
                    self.marbles[j].was_active = true;
                }

                // Activate blocks overlapping with the new displacement segment
                if segment_bounds.active {
                    let block_min_x = segment_bounds.min_x / block_size;
                    let block_max_x = (segment_bounds.max_x / block_size).min(cols - 1);
                    let block_min_y = segment_bounds.min_y / block_size;
                    let block_max_y = (segment_bounds.max_y / block_size).min(rows - 1);
                    for by in block_min_y..=block_max_y {
                        for bx in block_min_x..=block_max_x {
                            self.last_displacements[by * cols + bx] = 1.0;
                        }
                    }
                }
            } else {
                self.marbles[j].was_active = false;
            }

            // Sync with primary fields for backward compatibility
            if j == 0 {
                self.marble_pos = self.marbles[0].pos;
                self.prev_marble_pos = self.marbles[0].prev_pos;
                self.marble_vel = self.marbles[0].vel;
                self.was_active = self.marbles[0].was_active;
            }
        }



        // Run the gravity-driven settling cellular automata tick
        let has_active = self.last_displacements.iter().any(|&x| x > 3e-4)
            || self.marbles.iter().any(|m| m.was_active)
            || self.gravity_dir.length_squared() > 1e-6;
        if has_active {
            let mut active_marbles = [physics::ActiveMarbleInfo {
                pos: Vec2::ZERO,
                vel: 0.0,
                vel_vec: Vec2::ZERO,
            }; 5];
            let mut active_count = 0;
            for j in 0..5 {
                if self.marbles[j].was_active {
                    let m_vel_vec = if dt > 1e-5 { self.marbles[j].vel / dt } else { Vec2::ZERO };
                    active_marbles[active_count] = physics::ActiveMarbleInfo {
                        pos: self.marbles[j].pos,
                        vel: m_vel_vec.length(),
                        vel_vec: m_vel_vec,
                    };
                    active_count += 1;
                }
            }

            let fresh_active = physics::compute_fresh_active(
                w,
                h,
                block_size,
                cols,
                rows,
                &self.shape_mask,
                &self.heightmap.data,
                &self.heightmap.external_mass_this_tick,
                &self.cell_props,
                &self.edge_vel_v,
                &self.last_displacements,
            );
            settle_tick(
                    &mut self.heightmap,
                    &mut self.temp_heights,
                    &mut self.cell_colors,
                    &mut self.cell_props,
                    &mut self.sliding,
                    &mut self.active_bounds,
                    &mut self.active_blocks,
                    &mut self.last_displacements,
                    &mut self.last_simulated_ticks,
                    self.budget_n,
                    self.block_size,
                    &active_marbles[..active_count],
                    time_seed,
                    &mut self.edge_vel_h,
                    &mut self.edge_vel_v,
                    &mut self.column_depth,
                    &self.shape_mask,
                    self.tick_count,
                    self.gravity_dir,
                    // CLASSIFICATION-HOIST.md Stage 1: computed once, just above.
                    Some(&fresh_active),
                    self.liquid_fall_jitter,
                    self.lateral_substeps,
                );
        } else {
            self.active_bounds.active = false;
        }

        self.tick_count = self.tick_count.wrapping_add(1);

        // Quantile mass-distribution lines (Sand-fall overlay): the steady-state path recomputes
        // at most every 5 ticks, and only while the feature is switched on. `has_active` being
        // false means nothing moved this tick, so row_mass couldn't have changed either — skip in
        // that case too.
        //
        // That per-5-tick path alone is not enough: `refresh_quantiles_partial` only re-sums rows
        // whose block-row is active in the exact tick sampled, so a row that changed and then went
        // INACTIVE again on a tick this gate never lands on keeps a stale cached mass forever —
        // see `QUANTILE_FULL_RESYNC_TICKS`'s doc comment. So every `QUANTILE_FULL_RESYNC_TICKS`
        // ticks we pay one full recompute regardless of `has_active`, deliberately *not*
        // has_active-gated, because the whole point is to catch mass that changed on a tick this
        // tick's activity snapshot cannot see.
        //
        // This is the whole cost-gating story: when `quantile_mode == Off` (the default), none of
        // this — the every-5-tick partial re-sum or the every-100-tick full recompute — ever runs.
        if self.quantile_mode != QuantileMode::Off {
            if self.tick_count % QUANTILE_FULL_RESYNC_TICKS == 0 {
                self.refresh_quantiles_full();
            } else if has_active && self.tick_count % 5 == 0 {
                self.refresh_quantiles_partial();
            }
        }

        // Update EMA of frame time and adjust budget_n
        const EMA_ALPHA: f32 = 0.1;
        // Derived from the block count rather than hardcoded: these are block-count throttles,
        // and the block count is resolution-dependent again (`DEFAULT_BLOCK_SIZE`). The divisors
        // in `budget_throttles` reproduce the previous absolute 128 / 16 / 4 exactly at grid 512.
        let budget_max = cols * rows; // 4096 at grid 512
        let (_, budget_min, budget_step_down, budget_step_up) = budget_throttles(budget_max);

        if last_frame_time_ms > 0.0 && target_frame_time_ms > 0.0 {
            self.ema_frame_ms = EMA_ALPHA * last_frame_time_ms + (1.0 - EMA_ALPHA) * self.ema_frame_ms;

            // Target 95% of target FPS (Vsync interval * 1.05) to account for browser Vsync-locking
            // and allow the budget to grow back up when running smoothly.
            let adjusted_target = target_frame_time_ms * 1.05;

            if self.ema_frame_ms > adjusted_target {
                self.budget_n = self.budget_n.saturating_sub(budget_step_down).max(budget_min);
            } else if self.ema_frame_ms < adjusted_target {
                self.budget_n = (self.budget_n + budget_step_up).min(budget_max);
            }
        }
    }
}

impl HeightmapSimulation for DrawingSimulation {
    fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]) {
        let mut targets = [None; 5];
        for (i, target) in cursor_targets.iter().take(5).enumerate() {
            targets[i] = *target;
        }
        let radius = self.marble_radius;
        let mat = self.material_mode;
        let shape = self.sandbox_shape;
        self.update(dt, &targets, radius, mat, shape, dt * 1000.0, dt * 1000.0);
    }

    fn reset(&mut self) {
        self.reset();
    }

    fn heightmap(&self) -> &[f32] {
        self.heightmap.as_slice()
    }

    fn dimensions(&self) -> (usize, usize) {
        (self.heightmap.width, self.heightmap.height)
    }

    fn marbles(&self) -> &[MarbleState; 5] {
        &self.marbles
    }

    fn active_bounds(&self) -> ActiveBounds {
        self.active_bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape offered under the "Sand-fall Funnels" group in the UI. Kept in one place so a
    /// new funnel is covered by the geometry and mass-conservation tests by default rather than
    /// by remembering to add it to each.
    const SANDFALL_FUNNEL_SHAPES: [SandboxShape; 7] = [
        SandboxShape::Hourglass,
        SandboxShape::GaltonBoard,
        SandboxShape::StaircaseCascade,
        SandboxShape::ProceduralFunnel,
        SandboxShape::MultiNeckHourglass,
        SandboxShape::UTubeFlowThrough,
        SandboxShape::ChamberNetwork,
    ];

    /// Every material's string id must round-trip through `from_str`/`as_str`, and `ALL` must
    /// list each variant exactly once. This is the guarantee the web UI's material `<select>`
    /// relies on: it builds its options from `MaterialMode::ALL` (via `list_materials`) and
    /// sends the id straight back through `from_str` on selection, so a stable id-per-variant
    /// with no duplicates/gaps is what keeps a selection pointing at the right material — the
    /// exact property that a past UI rewrite silently broke when materials were keyed by array
    /// index instead.
    #[test]
    fn test_material_mode_string_ids_round_trip() {
        use std::collections::HashSet;
        let mut seen_ids = HashSet::new();
        for mode in MaterialMode::ALL {
            let id = mode.as_str();
            assert!(seen_ids.insert(id), "duplicate material id: {}", id);
            assert_eq!(
                MaterialMode::from_str(id),
                Some(mode),
                "round-trip failed for {:?} -> {:?}",
                mode,
                id
            );
        }
        assert_eq!(seen_ids.len(), MaterialMode::ALL.len());
        assert_eq!(MaterialMode::from_str("not_a_real_material"), None);
    }

    #[test]
    fn test_simulation_reset() {
        let mut sim = DrawingSimulation::new();
        sim.marble_pos = Vec2::new(0.5, -0.5);
        sim.heightmap.set(100, 100, 0.0);
        sim.reset();
        assert_eq!(sim.marble_pos, Vec2::ZERO);
        let val = sim.heightmap.get(100, 100);
        assert!((val - DEFAULT_SAND_HEIGHT).abs() < 0.035);
    }

    #[test]
    fn test_norm_to_grid_mapping() {
        let width = 512;
        let height = 512;

        // Verify corners map to exact boundary indexes
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-1.0, 1.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(1.0, -1.0), width, height),
            (width - 1, height - 1)
        );

        // Verify center mapping falls in correct bins (256, 256)
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(0.0, 0.0), width, height),
            (256, 256)
        );

        // Verify bounds clamping maps out of bounds coordinates to grid edges safely
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(-2.0, 2.0), width, height),
            (0, 0)
        );
        assert_eq!(
            DrawingSimulation::norm_to_grid(Vec2::new(2.0, -2.0), width, height),
            (width - 1, height - 1)
        );
    }

    #[test]
    fn test_norm_to_grid_nan_inf() {
        let width = 512;
        let height = 512;

        // NAN should map safely without panic
        let nan_pos = Vec2::new(f32::NAN, f32::NAN);
        let (x, y) = DrawingSimulation::norm_to_grid(nan_pos, width, height);
        assert!(x < width && y < height);

        // Inf should map safely without panic
        let inf_pos = Vec2::new(f32::INFINITY, f32::NEG_INFINITY);
        let (x, y) = DrawingSimulation::norm_to_grid(inf_pos, width, height);
        assert!(x < width && y < height);
    }

    #[test]
    fn test_marble_movement_noise_and_drift() {
        let mut sim = DrawingSimulation::new();
        let mut targets = [None; 5];
        // Initially target is None, should not be active
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(!sim.was_active);

        // Move to start point (first point is exact target)
        targets[0] = Some(Vec2::new(0.1, 0.2));
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);
        assert!(sim.was_active);
        assert_eq!(sim.marble_pos, Vec2::new(0.1, 0.2));

        // Move to next point, introducing noise, drag, and jitter
        let target = Vec2::new(0.3, 0.4);
        targets[0] = Some(target);
        sim.update(0.016, &targets, 0.025, MaterialMode::ButterCream, SandboxShape::Circle, 16.0, 16.0);

        // Ensure marble position shifted from start and is not exactly the target due to physics drift/noise
        assert_ne!(sim.marble_pos, Vec2::new(0.1, 0.2));
        assert_ne!(sim.marble_pos, target);

        // Verify that it is close to target but slightly drifted/jittered (less than 0.1 delta)
        let dist = (sim.marble_pos - target).length();
        assert!(dist < 0.1);

        // Verify marble velocity is populated
        assert_ne!(sim.marble_vel, Vec2::ZERO);
    }

    #[test]
    fn test_sandbox_shapes_clamping() {
        // Test Circle clamping: length should be clamped to max_r
        let p_circle = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Circle, 0.018);
        assert!((p_circle.length() - (0.92 - 0.018)).abs() < 1e-5);

        // Test Square clamping: X and Y should be clamped to max_r
        let p_square = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.5, 0.2), SandboxShape::Square, 0.018);
        assert_eq!(p_square.x, 0.92 - 0.018);
        assert_eq!(p_square.y, 0.2);

        // Test Oval clamping: should satisfy ellipse equation
        let p_oval = DrawingSimulation::clamp_to_sandbox(Vec2::new(1.0, 1.0), SandboxShape::Oval, 0.018);
        let a = 0.92 - 0.018;
        let b = 0.60 - 0.018;
        let d_sq = (p_oval.x * p_oval.x) / (a * a) + (p_oval.y * p_oval.y) / (b * b);
        assert!((d_sq - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_simulation_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            assert!(diff < 5e-3, "Step {}: Volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_multi_marble_large_spiral_volume_preservation() {
        let mut sim = DrawingSimulation::new();
        let initial_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();

        let mut targets = [None; 5];
        // Large marble radius
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
            
            let current_sum: f64 = sim.heightmap.data.iter().map(|&x| x as f64).sum();
            let diff = (current_sum - initial_sum).abs();
            // Use 2e-2 threshold for multi-marble large updates, due to larger accumulated float rounding errors.
            assert!(diff < 2e-2, "Step {}: Multi-marble volume leaked! diff = {}, initial = {}, current = {}", i, diff, initial_sum, current_sum);
        }
    }

    #[test]
    fn test_simulation_color_preservation() {
        let mut sim = DrawingSimulation::new();
        
        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = color_channel(s.cell_colors[idx], 0) as f64;
                let g = color_channel(s.cell_colors[idx], 1) as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        // Move marble in a spiral over 200 steps
        for i in 0..200 {
            let angle = i as f32 * 0.1;
            let radius = i as f32 * 0.004;
            targets[0] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            sim.update(
                0.016,
                &targets,
                0.018,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5% (to account for u8 integer rounding at each step)
        assert!(diff_red < 0.005, "Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }

    #[test]
    fn test_multi_marble_large_spiral_color_preservation() {
        let mut sim = DrawingSimulation::new();

        // Initialize cell_colors with a gradient/pattern
        let mut initial_colors = vec![0u8; GRID_SIZE * GRID_SIZE * 4];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = y * GRID_SIZE + x;
                initial_colors[idx * 4 + 0] = (x % 256) as u8;
                initial_colors[idx * 4 + 1] = (y % 256) as u8;
                initial_colors[idx * 4 + 2] = 128;
                initial_colors[idx * 4 + 3] = 255;
            }
        }
        sim.set_cell_colors(&initial_colors);

        let calculate_color_mass = |s: &DrawingSimulation| -> (f64, f64) {
            let mut red_mass = 0.0f64;
            let mut green_mass = 0.0f64;
            for (idx, &h) in s.heightmap.data.iter().enumerate() {
                let r = color_channel(s.cell_colors[idx], 0) as f64;
                let g = color_channel(s.cell_colors[idx], 1) as f64;
                red_mass += r * h as f64;
                green_mass += g * h as f64;
            }
            (red_mass, green_mass)
        };

        let (initial_red, initial_green) = calculate_color_mass(&sim);

        let mut targets = [None; 5];
        let marble_radius = 0.08;
        
        // Move 3 marbles in out-of-phase spirals over 150 steps
        for i in 0..150 {
            for j in 0..3 {
                let angle = i as f32 * 0.15 + (j as f32 * 2.0 * std::f32::consts::PI / 3.0);
                let radius = i as f32 * 0.005;
                targets[j] = Some(Vec2::new(angle.cos() * radius, angle.sin() * radius));
            }
            sim.update(
                0.016,
                &targets,
                marble_radius,
                MaterialMode::DrySand,
                SandboxShape::Circle,
                16.0,
                16.0,
            );
        }

        let (final_red, final_green) = calculate_color_mass(&sim);

        let diff_red = (final_red - initial_red).abs() / initial_red;
        let diff_green = (final_green - initial_green).abs() / initial_green;

        // Verify that the color mass is preserved within 0.5%
        assert!(diff_red < 0.005, "Multi-marble Red color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_red * 100.0, initial_red, final_red);
        assert!(diff_green < 0.005, "Multi-marble Green color mass leaked! diff = {:.5}%, initial = {}, final = {}", diff_green * 100.0, initial_green, final_green);
    }

    #[test]
    // Every funnel geometry has the same failure mode — a shelf, peg or neck that does not quite
    // close lets sand cross into MASK_OUTSIDE, where `settle_tick`'s mask guards freeze it
    // permanently — so all of them are worth the same check, and the ones whose geometry just
    // changed most of all: the Galton peg lattice, the three-neck hourglass and the finer
    // staircase.
    //
    // Run at the default neck width only — sweeping the slider here costs 20s of suite time, and
    // what the slider actually threatens is *geometric* (necks merging, shelves fusing into a
    // slab). That is covered per-shape instead, for free, by mask inspection:
    // `test_staircase_steps_stay_separated`.
    fn test_all_sandfall_funnels_conserve_sand_mass() {
        for shape in SANDFALL_FUNNEL_SHAPES {
            let mut sim = super::DrawingSimulation::new();
            sim.sandbox_shape = shape;
            sim.gravity_dir = Vec2::new(0.0, 0.04);
            sim.initialize_hourglass();

            let initial_mass: f32 = sim.heightmap.data.iter().sum();
            assert!(initial_mass > 0.0, "{:?}: initialized with no sand at all", shape);

            let targets = [None; 5];
            for _ in 0..300 {
                sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, shape, 16.0, 16.0);
            }

            let final_mass: f32 = sim.heightmap.data.iter().sum();
            let mass_err = (final_mass - initial_mass).abs() / initial_mass;
            assert!(
                mass_err < 0.0001,
                "{:?}: leaked sand through the geometry. init={:.4} final={:.4} err={:.6}",
                shape, initial_mass, final_mass, mass_err
            );
        }
    }

    #[test]
    // The geometric companion to the mass test above: pure mask inspection, so it costs nothing
    // to run. This one covers StaircaseCascade only, at the default neck width — the staircase's
    // geometry does not depend on the neck slider.
    //
    // The failure it exists for is the staircase. Consecutive shelves alternate slope sign and
    // which wall they attach to, so they converge at the shared inner edge; reduce the step
    // spacing without reducing the slope to match and neighbouring shelves fuse into one thick
    // slab. Sand still gets past — every shelf leaves an open side — so this is not a leak and
    // the mass test above sails straight through it. What is lost is the staircase itself: ask
    // for 13 steps, see six fat ones.
    //
    // Measured as the thickest unbroken run of wall down any column. A single shelf is 7 cells
    // (half-thickness 3.5 either side of its centre line); a fused pair is twice that. There is
    // no ambiguity between the two — measured on the shipped grid, the 0.04..0.08 slope gives a
    // maximum run of exactly 7 and the old 0.10..0.20 slope at this step count gives exactly 14,
    // at dx = -102, right where the model above says the two shelves cross.
    //
    // Note it has to scan the full width, not the middle. Consecutive shelves are separated by
    // `step_spacing - 2 * dx * slope`, which is at its largest on the axis and only closes near
    // the attach edge at dx ~ +/-98 — sampling near the centre, as an earlier version of this
    // test did, reports every configuration as healthy including one with shelves visibly fused.
    fn test_staircase_steps_stay_separated() {
        let mut sim = super::DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::StaircaseCascade;
        sim.generate_shape_mask();

        let w = GRID_SIZE;
        let h = GRID_SIZE;
        // Scanning only between the first and last row holding any interior keeps the box's own
        // top and bottom casing out of the measurement; within that band, at any column, the only
        // wall is shelf.
        let occupied: Vec<usize> = (0..h)
            .filter(|&y| (0..w).any(|x| sim.shape_mask[y * w + x] != MASK_OUTSIDE))
            .collect();
        let (first, last) = (occupied[0], *occupied.last().unwrap());

        let mut worst_run = 0usize;
        let mut worst_x = 0usize;
        for x in 0..w {
            if !(first..=last).any(|y| sim.shape_mask[y * w + x] != MASK_OUTSIDE) {
                continue; // column is entirely outside the box
            }
            let mut run = 0usize;
            for y in first..=last {
                if sim.shape_mask[y * w + x] == MASK_OUTSIDE {
                    run += 1;
                    if run > worst_run {
                        worst_run = run;
                        worst_x = x;
                    }
                } else {
                    run = 0;
                }
            }
        }

        assert!(
            worst_run <= 10,
            "StaircaseCascade has a {}-cell-thick wall run at x={} (dx={}); one shelf is 7, so \
             consecutive shelves have fused into a slab and the cascade has fewer, fatter steps \
             than the 13 configured",
            worst_run,
            worst_x,
            worst_x as i32 - (w as i32 / 2)
        );
    }

    #[test]
    // Flipping the apparatus must invert the *structure*, not only its contents. The mask used
    // to be left untouched, so an asymmetric shape kept its original orientation while the sand
    // mirrored into it — shelves that had been catching sand were suddenly upside down relative
    // to the pile sitting on them.
    //
    // Checked structurally rather than by running sand: the flipped mask must equal the upright
    // mask mirrored about `center_y = h / 2`, which is the same axis `flip_hourglass` mirrors the
    // contents about (`y2 = h - y`). If those two axes ever drift apart the sand lands inside the
    // walls, so this pins them together.
    fn test_flip_inverts_the_structure_not_just_the_sand() {
        for shape in [
            SandboxShape::StaircaseCascade,
            SandboxShape::ProceduralFunnel,
            // ChamberNetwork's routing tables are asymmetric by design (R1's `[(0,1),(1,2),
            // (2,3),(3,2)]` has no left-right symmetry), so it belongs in this structural-flip
            // check for the same reason as the other two: an asymmetric shape is exactly what
            // would silently keep its original orientation if `generate_shape_mask()` were ever
            // skipped from `flip_hourglass()` again.
            SandboxShape::ChamberNetwork,
        ] {
            let mut sim = super::DrawingSimulation::new();
            sim.sandbox_shape = shape;
            sim.generate_shape_mask();
            let upright = sim.shape_mask.clone();

            sim.flip_hourglass();
            let flipped = sim.shape_mask.clone();

            let w = GRID_SIZE;
            let h = GRID_SIZE;
            let (mut compared, mut mismatched) = (0usize, 0usize);
            for y in 1..h {
                for x in 0..w {
                    compared += 1;
                    if flipped[y * w + x] != upright[(h - y) * w + x] {
                        mismatched += 1;
                    }
                }
            }
            assert_eq!(
                mismatched, 0,
                "{:?}: flipped mask is not the mirror of the upright one ({} of {} cells differ)",
                shape, mismatched, compared
            );

            // ...and the flip has to be a real change for these shapes, or the assertion above
            // would pass just as happily against a mask that never moved.
            let differs = upright.iter().zip(&flipped).filter(|(a, b)| a != b).count();
            assert!(
                differs > 0,
                "{:?}: mask is identical after flipping, so nothing was actually inverted",
                shape
            );

            // Flipping twice returns to the original orientation.
            sim.flip_hourglass();
            assert_eq!(
                sim.shape_mask, upright,
                "{:?}: two flips did not return the structure to upright",
                shape
            );
        }
    }

    #[test]
    fn test_quantile_mode_off_by_default_and_costs_nothing() {
        let sim = DrawingSimulation::new();
        assert_eq!(sim.quantile_mode, QuantileMode::Off);
        assert!(sim.quantile_positions().is_empty());
    }

    #[test]
    fn test_quantile_positions_stay_empty_while_off_during_hourglass_run() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();

        // 20 ticks is plenty: the assertion is only that the mode gate never fires, and the
        // refresh is scheduled every 5 ticks, so this still covers several would-be refreshes.
        let targets = [None; 5];
        for _ in 0..20 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 16.0, 16.0);
        }

        // Never opted in: no positions should ever be computed.
        assert!(sim.quantile_positions().is_empty());
    }

    #[test]
    // A Galton board only does anything if every grain is forced to hit a peg and pick a side.
    // Sand used to fall straight through it in visible vertical lines, for two compounding
    // reasons, both of which this pins:
    //
    //  1. The row stagger was a no-op. Rows were centred on their own peg count, and
    //     `(count - 1) / 2` with `count = row + 3` is a half-integer on exactly the odd rows —
    //     the same rows an explicit `spacing * 0.5` offset shifted — so the two cancelled and
    //     every peg of every row landed on a multiple of the spacing.
    //  2. Even staggered, the pegs were too small to close the gap: the union of two rows offset
    //     by `s / 2` covers the line only when the radius is at least `s / 4`, and the radius was
    //     1.8 against a spacing of 8.
    //
    // The metric is what the user actually sees: a column of the board with no obstruction
    // anywhere down it. Measured on the shipped geometry before the fix, four such shafts about
    // 4.2 cells wide sat between every pair of peg columns.
    // Re-run at every "Simulation downscale" size the app offers (64/128/256/512, i.e.
    // `scale = w / 512` in {1/8, 1/4, 1/2, 1}) -- the GaltonBoard arm of `eval_sandbox_shape`
    // scales its peg lattice with `w` (2026-09-14) precisely so this holds at every S, not just
    // the shipped default. Assertion per size is unchanged from the original single-512 version.
    fn test_galton_board_has_no_clear_vertical_shafts() {
        for w in [64usize, 128, 256, 512] {
            let mut sim = super::DrawingSimulation::new_with_size(w);
            sim.sandbox_shape = SandboxShape::GaltonBoard;
            sim.generate_shape_mask();

            let h = w;
            // The peg field lives below the neck, in `dy` in (field_start, 0.38 * h) — see the
            // GaltonBoard arm of `eval_sandbox_shape`. Sample the interior of that band only, so
            // the funnel's own taper cannot be mistaken for an obstruction.
            let y_lo = h / 2 + 8;
            let y_hi = h / 2 + (0.34 * h as f32) as usize;

            let mut open_shafts = Vec::new();
            for x in 0..w {
                // Only columns that are actually open at the top of the band can be a shaft; a
                // column buried in the wall is not sand's path.
                if sim.shape_mask[y_lo * w + x] == MASK_OUTSIDE {
                    continue;
                }
                let blocked = (y_lo..y_hi).any(|y| sim.shape_mask[y * w + x] == MASK_OUTSIDE);
                if !blocked {
                    open_shafts.push(x);
                }
            }

            assert!(
                open_shafts.is_empty(),
                "w={}: sand falls straight through the Galton board at {} column(s) {:?} — no peg \
                 obstructs them anywhere between rows {} and {}",
                w,
                open_shafts.len(),
                open_shafts,
                y_lo,
                y_hi
            );

            // "No open shaft" alone missed the actual regression a spacing floor of 2 cells
            // caused (2026-09-14): at spacing 2 on the half-integer x axis, `dx mod spacing` has
            // only two residues (+-0.5, the SAME for every column), so any peg radius above 0.5
            // seals an ENTIRE row at once rather than leaving gaps between discrete pegs -- every
            // column in that row reports "blocked", so no column is ever an open SHAFT, but sand
            // cannot get through the row either. Assert directly against that failure mode: no
            // row in the peg band may have zero INSIDE/BOUNDARY cells across the whole width.
            let mut sealed_rows = Vec::new();
            for y in y_lo..y_hi {
                let any_inside = (0..w).any(|x| sim.shape_mask[y * w + x] != MASK_OUTSIDE);
                if !any_inside {
                    sealed_rows.push(y);
                }
            }
            assert!(
                sealed_rows.is_empty(),
                "w={}: peg band row(s) {:?} have NO inside/boundary cell anywhere across the \
                 width — the board is sealed there, not just missing a stray obstruction",
                w,
                sealed_rows
            );

            // Stronger than both checks above: a 4-neighbour flood fill over every non-OUTSIDE
            // cell, restricted to the peg band's own rows, must connect the band's top row to its
            // bottom row. Neither "no open shaft" nor "no sealed row" alone rules out a board
            // that is locally open everywhere but globally disconnected (e.g. two isolated
            // pockets with no path between them) -- this is the actual "can sand get from the top
            // of the peg field to the bottom" property the previous two are proxies for.
            let band_h = y_hi - y_lo;
            let mut visited = vec![false; w * band_h];
            let mut stack: Vec<(usize, usize)> = Vec::new();
            for x in 0..w {
                if sim.shape_mask[y_lo * w + x] != MASK_OUTSIDE {
                    visited[x] = true;
                    stack.push((x, y_lo));
                }
            }
            while let Some((x, y)) = stack.pop() {
                let candidates = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in candidates {
                    if nx < w && ny >= y_lo && ny < y_hi {
                        let vi = (ny - y_lo) * w + nx;
                        if !visited[vi] && sim.shape_mask[ny * w + nx] != MASK_OUTSIDE {
                            visited[vi] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            let bottom_row = y_hi - 1;
            let bottom_connected = (0..w).any(|x| visited[(bottom_row - y_lo) * w + x]);
            assert!(
                bottom_connected,
                "w={}: no path of INSIDE/BOUNDARY cells connects the top of the peg band (row \
                 {}) to its bottom (row {}) — the board is disconnected even though no single \
                 column or row check above caught it",
                w, y_lo, bottom_row
            );
        }
    }

    #[test]
    fn test_quantile_lines_descend_as_hourglass_drains() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();
        sim.set_quantile_mode(QuantileMode::Quartiles);

        // Immediately after init (all mass in the upper chamber), the median line should be
        // some finite position sitting up in the top half of the grid.
        let initial = sim.quantile_positions().to_vec();
        assert_eq!(initial.len(), 3);
        for &p in &initial {
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
        }
        // Ordered ascending (25% above 50% above 75%, all descending together over time).
        assert!(initial[0] <= initial[1] && initial[1] <= initial[2]);

        let targets = [None; 5];
        for _ in 0..200 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 16.0, 16.0);
        }

        let later = sim.quantile_positions().to_vec();
        assert_eq!(later.len(), 3);
        for &p in &later {
            assert!(p.is_finite() && (0.0..=1.0).contains(&p));
        }
        assert!(later[0] <= later[1] && later[1] <= later[2]);

        // As sand drains from the upper chamber into the lower one, every quantile line should
        // have moved further down the grid (larger normalised position) — mesmerizing descent,
        // not sideways drift or staying put.
        for (i, (&before, &after)) in initial.iter().zip(later.iter()).enumerate() {
            assert!(
                after > before,
                "quantile line {} should have descended: before={}, after={}",
                i,
                before,
                after
            );
        }
    }

    #[test]
    // Regression test for a reported bug: with Deciles active on a draining Hourglass, one
    // quantile line stayed pinned at the top instead of descending like every other line. The
    // scan in `compute_quantile_positions` is a true cumulative-mass walk from row 0, so a small
    // stranded remnant cannot pin a line by itself -- the scan would walk past a thin remnant and
    // follow the pile down. The real defect is upstream: `row_mass` itself goes stale, because
    // `refresh_row_mass_active` only re-sums a row when some block in its block-row is active *in
    // the exact tick sampled* (every 5th tick), and that snapshot is not an OR across the ticks in
    // between. A row a block touched on ticks N+1..N+4 and then went INACTIVE on N+5 (or any later
    // unsampled tick) keeps its old, too-high cached mass forever -- inflating a thin remnant into
    // a phantom double-digit percentage of the total.
    //
    // This must be caught by comparing the cached row_mass to a from-scratch recompute over the
    // *same* heights, not by asserting the lines merely move: `test_quantile_lines_descend_as_hourglass_drains`
    // already asserts movement and already passes, because most lines genuinely do move even with
    // this bug present -- only the specific stale row is wrong, which a "did it move" check cannot
    // see.
    fn test_row_mass_cache_does_not_go_stale_after_blocks_deactivate() {
        // Matches the user's exact repro: Deciles + Circle + Sand-fall gravity.
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Circle;
        sim.gravity_dir = Vec2::new(0.0, SANDFALL_GRAVITY_STRENGTH);
        sim.reset();
        sim.set_quantile_mode(QuantileMode::Deciles);

        let targets = [None; 5];
        // 300 ticks: comfortably enough for the upper chamber to drain down to a thin remnant and
        // for blocks near the top to fall fully INACTIVE well before the run ends, and an exact
        // multiple of `QUANTILE_FULL_RESYNC_TICKS` (100) so the fix's periodic full recompute has
        // just run on this very last tick -- making the cached row_mass and a fresh recompute
        // directly (not just approximately) comparable.
        for _ in 0..300 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Circle, 16.0, 16.0);
        }

        // Guard against the scenario going quiescent too early: if nothing of substance ever
        // moved, a stale cache and a correct one would trivially agree and this test would pass
        // vacuously (see docs/ARCHITECTURE.md section 11).
        let total_mass: f32 = sim.row_mass.iter().sum();
        assert!(
            total_mass > 1.0,
            "scenario went quiescent with almost no mass ({}) -- test would be vacuous",
            total_mass
        );

        let mut fresh_row_mass = Vec::new();
        refresh_row_mass_full(
            &sim.heightmap.data,
            sim.heightmap.width,
            sim.heightmap.height,
            &sim.shape_mask,
            &mut fresh_row_mass,
        );

        let mut worst_row = 0usize;
        let mut worst_diff = 0.0f32;
        for (y, (&cached, &fresh)) in sim.row_mass.iter().zip(fresh_row_mass.iter()).enumerate() {
            let diff = (cached - fresh).abs();
            if diff > worst_diff {
                worst_diff = diff;
                worst_row = y;
            }
        }

        assert!(
            worst_diff < 1e-4,
            "cached row_mass has gone stale at row {}: cached={}, fresh recompute={} (diff={}) \
             -- the periodic full resync should keep these in sync",
            worst_row,
            sim.row_mass[worst_row],
            fresh_row_mass[worst_row],
            worst_diff
        );
    }

    #[test]
    fn test_set_quantile_mode_off_clears_targets() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();
        sim.set_quantile_mode(QuantileMode::Deciles);
        assert_eq!(sim.quantile_positions().len(), 9);

        sim.set_quantile_mode(QuantileMode::Off);
        assert!(sim.quantile_positions().is_empty());
    }

    #[test]
    // Direct proof of the fix, independent of the long-run behavioural test below: after a
    // Circle reset (the non-Hourglass branch of `reset()`, which fills the *entire* grid via
    // `generate_smooth_noise` with no shape-mask zeroing), a large fraction of the raw
    // `heightmap.data` sum sits in cells outside the circular mask that the solver can never
    // reach. The row-mass cache the quantile scan reads must total to the masked sum, not the
    // raw one -- otherwise that phantom mass is exactly what pins an early decile line.
    fn test_quantile_row_mass_excludes_out_of_mask_phantom_mass_after_circle_reset() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Circle;
        sim.reset();

        let raw_total: f32 = sim.heightmap.data.iter().sum();
        let masked_total: f32 = sim
            .heightmap
            .data
            .iter()
            .zip(sim.shape_mask.iter())
            .filter(|&(_, &m)| m != MASK_OUTSIDE)
            .map(|(&h, _)| h)
            .sum();

        assert!(raw_total > 0.0, "reset() should leave some mass in the grid");
        let phantom_fraction = (raw_total - masked_total) / raw_total;
        assert!(
            phantom_fraction > 0.05,
            "expected a significant out-of-mask phantom fraction after Circle reset(), got {} \
             (raw_total={}, masked_total={}) -- if this ever goes to ~0, the separate zeroing fix \
             described in the task brief may already be in place and this test's premise no \
             longer holds",
            phantom_fraction,
            raw_total,
            masked_total
        );

        sim.set_quantile_mode(QuantileMode::Deciles);
        let row_mass_total: f32 = sim.row_mass.iter().sum();
        // Loose relative tolerance rather than a tight absolute one: row_mass_total is a sum of
        // 512 per-row partial sums (a different f32 reduction order than the flat sum used for
        // masked_total above), so the two accumulate rounding noise differently over ~262k
        // elements -- that's ordinary float32 summation-order noise, not a correctness gap. What
        // this assertion actually needs to rule out is `row_mass_total` including the raw,
        // unmasked total instead (a ~50% relative gap here), which this tolerance is nowhere near
        // wide enough to accidentally let through.
        let rel_err = (row_mass_total - masked_total).abs() / masked_total;
        assert!(
            rel_err < 1e-3,
            "quantile row_mass cache total should equal the mask-filtered sum, not the raw \
             unfiltered heightmap sum: row_mass_total={}, masked_total={}, raw_total={}, rel_err={}",
            row_mass_total,
            masked_total,
            raw_total,
            rel_err
        );
    }

    #[test]
    // Regression test for the user's exact reported bug: Deciles + Circle + Sand-fall gravity,
    // one decile line (the first, 10%-of-mass line) stayed pinned near the top of the grid while
    // every other decile line correctly descended as sand fell under gravity.
    //
    // Root cause: `refresh_row_mass_full`/`refresh_row_mass_active` summed raw `heightmap.data`
    // with no shape-mask filtering. `reset()`'s non-Hourglass branch -- which Circle, Square and
    // Oval all take -- fills the *entire* grid via `generate_smooth_noise`, including cells
    // outside the circular mask that the solver can never reach (every flux/CA path in
    // `physics.rs` is gated on `is_inside`), so those cells hold a frozen, never-updated height
    // forever. Counting that phantom height as live mass is enough on its own to satisfy an
    // early decile's cumulative-mass threshold before the scan ever reaches real, moving sand --
    // measured at 512, Circle's phantom fraction of the raw height-sum is ~0.335, comfortably
    // past the first decile's 0.1 threshold.
    //
    // This has to be caught by watching a line's position *change* over a long run, not by a
    // single snapshot -- a line sitting high up is not itself a bug, only a line that never moves
    // while its neighbours do. Sampling at t=50 and t=500 (well past `QUANTILE_FULL_RESYNC_TICKS`
    // = 100, so any staleness from that separate mechanism is not what's under test here) mirrors
    // the user's report of a decile line frozen across many hundreds of ticks.
    fn test_decile_lines_all_descend_for_circle_sandfall() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::Circle;
        sim.gravity_dir = Vec2::new(0.0, SANDFALL_GRAVITY_STRENGTH);
        // Circle takes reset()'s non-Hourglass, generate_smooth_noise branch -- the user's exact
        // repro path (not initialize_hourglass, which already zeroes out-of-mask cells).
        sim.reset();
        sim.set_quantile_mode(QuantileMode::Deciles);

        let targets = [None; 5];
        for _ in 0..50 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Circle, 16.0, 16.0);
        }
        let at_50 = sim.quantile_positions().to_vec();
        assert_eq!(at_50.len(), 9);

        for _ in 0..450 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Circle, 16.0, 16.0);
        }
        let at_500 = sim.quantile_positions().to_vec();
        assert_eq!(at_500.len(), 9);

        // Guard against the scenario going quiescent too early: if nothing of substance ever
        // moved, every line would trivially "stay put" and the assertions below would pass
        // vacuously (see docs/ARCHITECTURE.md section 11).
        let total_mass: f32 = sim.row_mass.iter().sum();
        assert!(
            total_mass > 1.0,
            "scenario went quiescent with almost no mass ({}) -- test would be vacuous",
            total_mass
        );

        eprintln!(
            "decile positions at t=50:  {:?}\ndecile positions at t=500: {:?}",
            at_50, at_500
        );

        // Every decile line -- including (especially) the first -- must have descended
        // (increased normalised position, since 0.0 = top row edge) by a meaningful amount. A
        // frozen phantom-mass cell pins a line's position exactly flat instead of letting it
        // track the real, moving pile underneath the phantom.
        let deltas: Vec<f32> = at_50.iter().zip(at_500.iter()).map(|(&b, &a)| a - b).collect();

        // Every decile line must have descended (increased normalised position) by a meaningful
        // amount in absolute terms. This alone is a weak check -- see below for why it is not
        // sufficient on its own.
        for (i, &delta) in deltas.iter().enumerate() {
            assert!(
                delta > 0.01,
                "decile line {} (cumulative fraction {}) should have descended between t=50 and \
                 t=500: before={}, after={}, delta={}",
                i,
                DECILE_FRACTIONS[i],
                at_50[i],
                at_500[i],
                delta
            );
        }

        // The discriminating check: decile line 0 must move by a substantial *fraction of* how
        // much the other eight lines moved, not merely by some small positive amount. A first
        // pass at this test used a bare `delta > 0.01` per line and it passed even with the bug
        // reverted (line 0 crept by 0.019, just clearing that bar, while lines 1-8 moved by
        // 0.06-0.41) -- exactly the vacuous-test trap the task brief warned about. The bug's
        // actual signature is line 0 moving a small fraction of what its neighbours do, not zero
        // movement, so the check has to be comparative rather than a small fixed floor.
        let others_mean_delta: f32 = deltas[1..].iter().sum::<f32>() / (deltas.len() - 1) as f32;
        assert!(
            deltas[0] > 0.3 * others_mean_delta,
            "decile line 0 barely moved (delta={}) relative to the mean movement of lines 1-8 \
             (mean delta={}) -- this is the reported bug's exact shape: one line pinned near the \
             top (by phantom, out-of-mask mass inflating its cumulative-mass threshold) while the \
             others correctly track the descending pile. deltas={:?}",
            deltas[0],
            others_mean_delta,
            deltas
        );
    }

    /// The shipped LOD geometry, pinned per resolution. `DEFAULT_BLOCK_SIZE` replaced the old
    /// `grid/64` divisor on 2026-09-02; nothing else in the suite goes through `new_with_size`,
    /// so without this the geometry has no coverage at all.
    #[test]
    fn test_shipped_block_geometry_is_a_constant_eight_cell_block() {
        // (grid, expected blocks per axis, expected block edge in cells)
        for (grid, axis, block_size) in [(64, 8, 8), (128, 16, 8), (256, 32, 8), (512, 64, 8)] {
            let divisor = default_block_divisor(grid);
            assert_eq!(divisor, axis, "grid {} should tile into {} blocks per axis", grid, axis);
            assert_eq!(
                (grid / divisor.max(1)).max(1),
                block_size,
                "grid {} should use a {}-cell block", grid, block_size
            );
        }

        // No resolution degenerates. `grid/64` gave block_size 1 at grid 64 (one block per cell,
        // the LOD scheduler doing nothing) and 2 at grid 128 (the slab artifact
        // `VERTICAL_PRESSURE_CAP_MULT` documents). Both are why that geometry went.
        for grid in [64usize, 128, 256, 512] {
            assert!(
                (grid / default_block_divisor(grid).max(1)).max(1) >= 4,
                "grid {} degenerates to a block smaller than 4 cells", grid
            );
        }

        // A future grid 1024 is capped at 64 blocks per axis rather than 128 -- a placeholder
        // bound, not a measured one. If 1024 ships, revisit `MAX_BLOCKS_PER_AXIS`.
        assert_eq!(default_block_divisor(1024), MAX_BLOCKS_PER_AXIS);
    }

    /// The adaptive controller's throttles are fractions of the block count now that the block
    /// count is resolution-dependent again. They must reproduce the previous hardcoded absolutes
    /// exactly at grid 512, the shipped `GRID_SIZE` -- that is what makes the geometry change a
    /// no-op at the default resolution.
    #[test]
    fn test_budget_throttles_match_the_old_absolutes_at_grid_512() {
        let blocks_512 = default_block_divisor(512) * default_block_divisor(512);
        assert_eq!(blocks_512, 4096);
        assert_eq!(budget_throttles(blocks_512), (1024, 128, 16, 4));

        // Never zero, however small the grid -- a zero step would freeze the controller and a
        // zero floor would let the budget collapse to nothing.
        for grid in [64usize, 128, 256, 512] {
            let blocks = default_block_divisor(grid) * default_block_divisor(grid);
            let (init, min, down, up) = budget_throttles(blocks);
            assert!(min >= 1 && down >= 1 && up >= 1, "grid {} produced a zero throttle", grid);
            assert!(init <= blocks, "grid {} initial budget exceeds the block count", grid);
            assert!(min <= init, "grid {} floor is above the initial budget", grid);
        }
    }

    /// CONTROL for the asymmetry hunt (2026-09-08): is the vessel MASK itself left-right
    /// symmetric? If it is not, no amount of solver symmetry can produce a symmetric result and
    /// the physics is innocent. Checked at every resolution the UI offers, for the shapes the
    /// asymmetry is reported in.
    ///
    /// `SandboxShape::ChamberNetwork` is deliberately NOT in this list. Mirror symmetry was
    /// waived for it by the user when the routing tables were approved: R1 (the default,
    /// `[(0,1),(1,2),(2,3),(3,2)]`) and R2 (`[(0,1),(0,2),(2,3),(3,2)]`, a cross) are both
    /// asymmetric by construction, and R5's two tables are asymmetric per-transition even
    /// though the shape as a whole is not. Requiring mirror symmetry here would mean rejecting
    /// the routing tables the user picked (2026-09-19, then redesigned 2026-09-23 to drop every
    /// pipe spanning more than one column -- see `NetworkRouting`'s doc comment).
    #[test]
    fn test_vessel_masks_are_left_right_symmetric() {
        let mut worst: Vec<String> = Vec::new();
        for grid in [128usize, 256, 512] {
            for (name, shape) in [
                ("Hourglass", SandboxShape::Hourglass),
                ("MultiNeckHourglass", SandboxShape::MultiNeckHourglass),
            ] {
                let mut sim = DrawingSimulation::new_with_size(grid);
                sim.sandbox_shape = shape;
                sim.generate_shape_mask();
                let w = grid;
                let mut mismatches = 0usize;
                let mut first: Option<(usize, usize)> = None;
                for y in 0..grid {
                    for x in 0..grid {
                        let m = sim.shape_mask[y * w + x];
                        let mirror = sim.shape_mask[y * w + (w - 1 - x)];
                        if m != mirror {
                            mismatches += 1;
                            if first.is_none() { first = Some((x, y)); }
                        }
                    }
                }
                if mismatches > 0 {
                    worst.push(format!(
                        "{} at grid {}: {} mirror mismatches (first at x={}, y={})",
                        name, grid, mismatches, first.unwrap().0, first.unwrap().1
                    ));
                }
                println!("MASKSYM {} grid={} mismatches={}", name, grid, mismatches);
            }
        }
        assert!(worst.is_empty(), "Vessel masks are not left-right symmetric:\n  {}", worst.join("\n  "));
    }

    /// `rasterize_shape_mask(out_size = S)` (what `generate_shape_mask` now calls) must be
    /// bit-identical to the mask this crate produced before that refactor: a discrete pass over
    /// `physics::eval_sandbox_shape`'s integer-cell API, computed independently right here rather
    /// than by calling `rasterize_shape_mask` a second time (which would just prove the function
    /// agrees with itself). Covers every `SandboxShape` at both ends of the resolution range the
    /// UI offers (64, 512) -- see `HANDOVER`/commit message for the same comparison re-run against
    /// the pre-refactor tree with an external checksum tool.
    #[test]
    fn test_rasterize_shape_mask_matches_discrete_eval_at_sim_size() {
        for w in [64usize, 512] {
            for shape in [
                SandboxShape::Circle,
                SandboxShape::Square,
                SandboxShape::Oval,
                SandboxShape::Hourglass,
                SandboxShape::GaltonBoard,
                SandboxShape::StaircaseCascade,
                SandboxShape::ProceduralFunnel,
                SandboxShape::MultiNeckHourglass,
                SandboxShape::UTubeFlowThrough,
                SandboxShape::ChamberNetwork,
            ] {
                let mut sim = DrawingSimulation::new_with_size(w);
                sim.sandbox_shape = shape;
                sim.neck_width = 0.06;
                sim.hourglass_curve = 0.8;
                sim.generate_shape_mask();

                // Independent discrete reimplementation of the old two-pass algorithm, using the
                // integer-cell `eval_sandbox_shape` entry point directly.
                let h = w;
                let mut expected = vec![MASK_OUTSIDE; w * h];
                for y in 0..h {
                    for x in 0..w {
                        let (inside, _safe) = physics::eval_sandbox_shape(
                            x, y, w, h, shape, sim.neck_width, sim.hourglass_curve,
                            sim.flipped, sim.network_routing,
                        );
                        expected[y * w + x] = if inside { MASK_INSIDE } else { MASK_OUTSIDE };
                    }
                }
                let snapshot = expected.clone();
                for y in 0..h {
                    for x in 0..w {
                        if snapshot[y * w + x] == MASK_INSIDE {
                            let has_outside_neighbor =
                                (x == 0 || snapshot[y * w + x - 1] == MASK_OUTSIDE) ||
                                (x + 1 >= w || snapshot[y * w + x + 1] == MASK_OUTSIDE) ||
                                (y == 0 || snapshot[(y - 1) * w + x] == MASK_OUTSIDE) ||
                                (y + 1 >= h || snapshot[(y + 1) * w + x] == MASK_OUTSIDE);
                            if has_outside_neighbor {
                                expected[y * w + x] = MASK_BOUNDARY;
                            }
                        }
                    }
                }

                assert_eq!(
                    sim.shape_mask, expected,
                    "{:?} at w={}: rasterize_shape_mask(S) diverged from the discrete-eval mask",
                    shape, w
                );
            }
        }
    }
}
