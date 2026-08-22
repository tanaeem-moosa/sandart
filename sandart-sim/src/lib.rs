pub mod coarse;
pub mod grid;
pub mod physics;
pub mod quantiles;

pub use coarse::{CoarseGeometry, CoarseState, COARSE_GRID};
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
pub fn early_term_log_take() -> Vec<(u32, i32)> {
    EARLY_TERM_LOG.with(|c| std::mem::take(&mut *c.borrow_mut()))
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

/// Height above which a cell counts as "holding material" for the "perfect simulation" debug
/// toggle's non-trivial-block scan (`DrawingSimulation::perfect_simulation`, `DrawingSimulation
/// ::update`). Not `0.0` exactly — draining can leave a cell at a sub-float residue that will
/// never itself flow anywhere, and forcing its block to simulate forever over dust like that
/// would turn the toggle's "every tick" promise into pointless busywork. Comfortably below
/// `physics::MUST_SIMULATE_THRESHOLD` (1e-4): this only decides whether a block is worth waking
/// up at all, not whether it's expected to move once it has.
const PERFECT_SIM_MATERIAL_EPSILON: f32 = 1e-5;

/// EARLY-STOP.md: per-block clock rate is now an ARBITRARY value in `[CLOCK_RATE_MIN,
/// CLOCK_RATE_MAX]` = `[1/8, 16]`, not quantised to a power of two.
///
/// The ceiling was 8 until rate GRADING landed (`grade_clock_rates`). Grading makes a high
/// ceiling self-limiting rather than dangerous: a block can only reach 16x if it sits in a fast
/// region wide enough to ramp there one step at a time from its surroundings, so raising the
/// ceiling grants headroom where the scene genuinely earns it instead of licensing isolated
/// blocks to sprint away from their neighbours. Without grading, treat 8 as the practical limit.
///
/// HIERARCHICAL-PRESSURE.md §7b's S1 justified the old power-of-two quantisation as needed "so
/// clock domains nest instead of beating" and so a shared edge never sees one side mid-step. That
/// reasoning does not apply to this implementation, and it was checked rather than assumed:
/// `update()` repeats whole `settle_tick` calls over a participation set, and every repetition is
/// a global synchronisation point -- a block either runs in that rep or it does not, atomically
/// (see `force_overclocked_blocks_active`). There is no partial per-substep state for two
/// non-nesting rates to desynchronise; a rate-3 block sitting out rep 3 while a rate-4 neighbour
/// runs is structurally identical to the old rate-2 block sitting out rep 1 while a rate-4
/// neighbour ran. S3 (edge ownership across a clock-rate boundary, below) is unchanged and is the
/// place this reasoning would show up if it were wrong -- it forces every neighbour of a running
/// block regardless of the neighbour's own rate, a mechanism that was always rate-value-agnostic.
///
/// The one residual risk §7b named -- a beat against the known period-2 checkerboard mode -- is
/// measurable, not theoretical: `vpar` in `diag_overclock_ab`'s oscillation measurement (the
/// production-resolution analogue of `overfill_pressure_toggle.rs`'s
/// `diag_task70_rest_color_mixing_and_checkerboard`) is the instrument. An earlier revision of
/// this comment claimed it had been run under arbitrary rates; it had not (EARLY-STOP.md records
/// that run being killed). It has now: `vpar` is -0.004 (Water) and -0.000 (DrySand) with
/// clocking on, against -0.000 / -0.002 with it off -- no parity split, so the beat §7b feared
/// does not appear. Settled CHURN is a separate reading and is NOT clean: Water churns
/// 0.000271/cell/tick clocked against 0.000029 unclocked and a 0.000025 at-rest baseline
/// (DrySand moves the other way, 0.000745 against 0.006274). That is measured under the
/// continuous rule only, with no quantised-rule control, so it is not yet attributable to
/// arbitrary rates -- and it is why `overclocking_enabled` stays default OFF.
///
/// With early stop (below) bounding a block's real repetitions at its own physical settle point,
/// `rate` is now a BUDGET, not a mandate, which is what makes a plain continuous rule -- no
/// hysteresis, no octave stepping -- safe to ship in place of the old one: the exact value matters
/// far less once physics, not the schedule, decides how many sub-steps a block actually gets.
const CLOCK_RATE_MIN: f32 = 0.125;
const CLOCK_RATE_MAX: f32 = 16.0;

/// The disagreement fraction (`|Delta[b]| / capacity[b]`, dimensionless) at which a block is
/// judged to want to run at the neutral 1x rate. `update_block_clock_rates` maps `signal /
/// CLOCK_DELTA_REF_FRAC` directly onto `rate`, continuously (EARLY-STOP.md: the old octave-stepped
/// hysteresis is gone, along with the power-of-two quantisation -- see `CLOCK_RATE_MIN`'s doc
/// comment for why removing both is safe here). Picked as a round, conservative number pending a
/// larger tuning pass -- see OVERCLOCKING.md for the rate distribution the old scheme produced at
/// this same reference fraction.
const CLOCK_DELTA_REF_FRAC: f32 = 0.05;

/// LATERAL-COARSE-CORRECTION.md: the shipped default for `coarse_correction_damping`.
///
/// **0.5, and this is a starting value rather than a measured optimum.** The reasoning for
/// starting at a half rather than at 1.0: the defect formulation makes 1.0 the *natural* value
/// ("the coarse level moved this much, the fine level moved that much, make up the difference"),
/// but the coarse level is not a Galerkin projection of the fine one -- different grid, and no
/// model of the angle of repose whatsoever -- so its answer carries real approximation error and
/// under-relaxing a coarse-grid correction under exactly those conditions is standard rather than
/// timid. A half also means an unstable interaction, if there is one, ratchets in over several
/// ticks instead of arriving whole on the first, which is the difference between a visible artifact
/// and a divergence.
///
/// `diag_lateral_corr` sweeps it. Whatever that sweep says should replace this number.
const COARSE_CORRECTION_DEFAULT_DAMPING: f32 = 0.5;

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
/// The shipped LOD block geometry: `block_size = grid/64`, which makes a block and a coarse
/// pressure tile the same square (`COARSE_GRID` is also 64). `new_with_block_divisor` exists to
/// vary this for measurement -- see BLOCK-SIZE-SWEEP.md.
const DEFAULT_BLOCK_DIVISOR: usize = 64;

const CLOCK_RATE_LADDER: [f32; 15] = [
    16.0, 14.0, 12.0, 10.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.5, 0.25, 0.125,
];

/// How strongly staleness (ticks since a block last ran) nudges its clock-rate signal upward.
/// Deliberately only ever able to push a block toward a HIGHER rate (never suppress one) --
/// "underclock conservatively" (HIERARCHICAL-PRESSURE.md §7b): a block that has gone a long time
/// without running is treated as more urgent, not less.
const CLOCK_STALENESS_WEIGHT: f32 = 0.03;

/// Number of coarse time-buckets the block-simulation heat-map overlay's trailing window is
/// divided into (see `DrawingSimulation::block_heat_buckets`). `HEAT_NUM_BUCKETS *
/// HEAT_BUCKET_TICKS` is the ~300-tick window the task asked for.
const HEAT_NUM_BUCKETS: usize = 10;
/// Ticks per heat-map bucket — see `HEAT_NUM_BUCKETS`.
const HEAT_BUCKET_TICKS: u32 = 30;

/// Fixed reference ceiling for the per-cell pressure-field heat-map overlay's log compression
/// (`DrawingSimulation::pressure_field_texels`). Chosen with headroom above the highest
/// `column_depth` measured in practice (464, for water 60 rows deep — see `column_depth`'s own
/// field doc comment in this file), so legitimate values compress smoothly toward 1.0 instead of
/// clipping in the common case.
///
/// This is a FIXED constant, not the current frame's own max (auto-normalisation): a fixed scale
/// means a given `column_depth` value always maps to the same on-screen colour, so two frames —
/// in particular the "Fresh pressure field" toggle's on/off states, which is exactly what this
/// overlay exists to compare — stay comparable by eye. The tradeoff is the opposite of what
/// auto-normalisation would give: a rarer value above this ceiling clips instead of the scale
/// stretching to fit it, and a frame whose whole field sits well below the ceiling (e.g. a mostly
/// empty grid) reads uniformly dim rather than being stretched to use the full range.
const PRESSURE_HEATMAP_LOG_MAX: f32 = 512.0;

/// How often the overfill heat-map's saturation deciles are recomputed, in ticks. These are a
/// LEGEND -- a scale the reader is reading off the screen -- so the requirement is legibility, not
/// freshness: boundaries that move every frame make the colour of a cell incomparable between two
/// consecutive frames and the overlay unreadable. 30 ticks is about half a second at 60fps, slow
/// enough to read and fast enough to follow a filling vessel.
const SATURATION_DECILE_REFRESH_TICKS: u32 = 30;

pub const PROP_WETNESS: usize = 0;
pub const PROP_THRESHOLD: usize = 1;
pub const PROP_FLOW_RATE: usize = 2;
pub const PROP_GRAIN_SIZE: usize = 3;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxShape {
    Circle,
    Square,
    Oval,
    Hourglass,
    MultiStageHourglass,
    GaltonBoard,
    StaircaseCascade,
    ProceduralFunnel,
    MultiNeckHourglass,
    /// Task #61: a U-shaped flow-through vessel -- a test apparatus for pressure work, not a
    /// sand cascade. Water fills the tall left reservoir arm, flows down through a partly
    /// ROOFED bottom basin (deliberately: this is the Pascal-pressure test case), climbs the
    /// shorter right arm, spills over its rim (the overflow lip) through a horizontal spout,
    /// and falls into a catch well. See `physics::U_TUBE_RECTS` for the geometry.
    UTubeFlowThrough,
}

impl Default for SandboxShape {
    fn default() -> Self {
        Self::Circle
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
    Oobleck,
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
    pub const ALL: [MaterialMode; 14] = [
        MaterialMode::DrySand,
        MaterialMode::KineticSand,
        MaterialMode::WetSand,
        MaterialMode::CoarseSand,
        MaterialMode::ButterCream,
        MaterialMode::Snow,
        MaterialMode::FinePowder,
        MaterialMode::Oobleck,
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
            MaterialMode::Oobleck => "oobleck",
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
            "oobleck" => MaterialMode::Oobleck,
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
            MaterialMode::Oobleck => "Oobleck",
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
            MaterialMode::Oobleck => (0.55, 0.04, 0.12, 0.15),
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
    pub cell_colors: Vec<u8>,
    /// Per-cell physics & render properties. Advected with height.
    /// Layout: [wetness, threshold, flow_rate, grain_size] interleaved.
    pub cell_props: Vec<f32>,
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
    /// Task #55 step 2 (rebuilt): the PERSISTENT hydraulic head field
    /// (`physics::task55_head_field::advance_head_field`). Unlike `column_depth`, this is not
    /// maintained every tick unconditionally -- `settle_tick` only advances it while EITHER
    /// `head_field_transport` OR `pressure_heatmap_head_field` is on (see those fields' own doc
    /// comments), since those are its only two consumers: the former reads it as the edge solvers'
    /// driving head, the latter reads it (via `head_field_to_pressure`) for the pressure heat-map
    /// overlay. Advancing it for the overlay alone does NOT make it feed transport -- the edge
    /// solvers' own gate is unchanged, tied only to `head_field_transport`/`head_field_gate` (see
    /// `settle_tick`'s `head_field_active` vs. `head_field_needs_advance`) -- so turning the
    /// overlay on cannot perturb the simulation, only what this buffer itself contains. While BOTH
    /// are off, this buffer simply holds whatever it was last relaxed to (or its zero-filled
    /// initial state, if neither has been turned on since the last reset/resize) -- it is never
    /// rebuilt from scratch on a per-read basis the way the deleted `compute_head_field` was; see
    /// `physics::task55_head_field`'s module doc comment for why that per-call-solve design was
    /// replaced. Read by `pressure_field_texels` when `pressure_heatmap_head_field` is set, and
    /// resized/zero-filled everywhere `column_depth` is (construction, `reset()`,
    /// `flip_hourglass()`, and any grid-size change via `set_grid_size`'s sim rebuild).
    pub head_field: Vec<f32>,
    /// Seed for marble movement noise.
    pub seed: u32,

    // Internal simulation configuration fields
    pub marble_radius: f32,
    pub material_mode: MaterialMode,
    pub sandbox_shape: SandboxShape,
    pub gravity_dir: Vec2,
    pub neck_width: f32,
    pub hourglass_curve: f32,
    /// The widest (top) tier's chamber count for `SandboxShape::MultiStageHourglass`'s
    /// merging cascade -- user-selectable 5..=16, default 8 (today's shipped, hard-coded
    /// value before this field existed). Every tier below is derived from this one number
    /// by `physics::multistage_tier_chambers`; see that function's doc comment for the
    /// merge rule. Setting this follows the same contract as `neck_width`/
    /// `hourglass_curve`: the setter (`sandart-wasm`'s `set_multistage_chambers`) just
    /// assigns the field and calls `generate_shape_mask()`, it does not reset the sim.
    pub multistage_chambers: u32,

    /// Precomputed shape mask grid (GRID_SIZE * GRID_SIZE).
    /// Values: MASK_OUTSIDE (0) = wall, MASK_INSIDE (1) = playable interior,
    /// MASK_BOUNDARY (2) = inside but adjacent to a wall cell.
    pub shape_mask: Vec<u8>,
    /// Set to true when the shape_mask has been regenerated and needs GPU re-upload.
    pub shape_mask_dirty: bool,
    /// Whether the apparatus is currently upside down. Consumed by `generate_shape_mask` (it
    /// negates `dy` in the shape evaluator), so the *structure* inverts along with its contents
    /// — asymmetric shapes like StaircaseCascade and the MultiStageHourglass cascade's tiers used to keep their
    /// original orientation while the sand mirrored into them. Stored rather than applied to the
    /// mask in place because the mask is rebuilt from scratch whenever neck width, curvature or
    /// the shape itself changes, which would silently discard an in-place mirror.
    pub flipped: bool,

    /// Coarse pressure geometry (`sandart_sim::coarse`, HIERARCHICAL-PRESSURE.md §4, build
    /// step 1). Rebuilt at the end of `generate_shape_mask` -- and ONLY there, so it is always
    /// exactly as fresh as `shape_mask` itself, never staler and never rebuilt on some other
    /// cadence.
    pub coarse: CoarseGeometry,

    /// Coarse simulation state (`sandart_sim::coarse`, HIERARCHICAL-PRESSURE.md §5, build
    /// step 2). Holds restricted fine mass A, persistent coarse mass M, coarse head eta,
    /// pressure P, and coarse-fine disagreement Delta.
    pub coarse_state: CoarseState,

    /// "Coarse pressure coupling" debug toggle (HIERARCHICAL-PRESSURE.md build step 3). Same
    /// shape as `overfill_pressure`/`head_field_transport`/`pressure_sensitive_flow`/
    /// `fresh_pressure_field` above: a plain UI-facing field, forwarded straight through, no
    /// reset. **Defaults to `true`**, unlike its siblings -- this is the one debug toggle in the
    /// group that ships ON, because the point of shipping it at all is for the coarse level to be
    /// visible by default, with the toggle available to switch it back off for an A/B comparison.
    ///
    /// `true` (default): today's coupled behaviour, unchanged -- `update()` runs
    /// `coarse_state.tick(...)` every tick (restrict / anchor / advance the nested coarse sim /
    /// export eta+delta) and `settle_tick` receives `&coarse_state.eta` / `&coarse_state.delta`
    /// whenever `coarse.available`, exactly as before this toggle existed.
    ///
    /// `false`: BOTH halves of the coupling are removed, not just one.
    /// - `update()` skips the `coarse_state.tick(...)` call entirely -- restriction, anchoring,
    ///   and the nested 64x64 solver step all stop running, so the toggle also measures the
    ///   coarse level's own per-tick cost, not just its effect on the fine grid.
    /// - `settle_tick` is called with empty `coarse_eta`/`coarse_delta` slices, the exact same
    ///   "not coupled" signal `coarse.available == false` already produces (see
    ///   `physics::coarse_delta_eta`'s doc comment) -- so `settle_tick` takes the identical
    ///   code path it took before this coupling existed. `coarse_state`'s own buffers (`eta`,
    ///   `delta`, `m_mass`, ...) are left exactly as they last were (frozen, not zeroed) while
    ///   off; they resume from wherever they were the instant this is flipped back on.
    ///
    /// "Coarse pressure coupling" debug toggle -- OVERCLOCKING.md split this field's meaning.
    /// It now gates ONLY the driving-POTENTIAL half of the coupling: whether `settle_tick`
    /// receives non-empty `coarse_eta`/`coarse_delta` (i.e. whether the coarse level's hydraulic
    /// head reaches `phi`/`gravity_head` on any fine edge at all). It no longer gates the coarse
    /// level's OWN per-tick dynamics -- `update()` now runs `coarse_state.tick(...)` (restrict /
    /// anchor / advance the nested coarse sim / export eta+delta) unconditionally whenever
    /// `coarse.available`, because the multi-rate block scheduler (`overclocking_enabled` below)
    /// needs `|Delta|` regardless of whether the potential coupling is on. This also means the
    /// coarse level's own per-tick cost is no longer part of what this toggle measures; it is now
    /// a fixed cost whenever `coarse.available`.
    ///
    /// **Defaults to `false`** (changed from `true`) -- the user's own words: "let's leave the
    /// coupling behind a flag until we are happy with overclocking." The coarse level runs and
    /// its `|Delta|` drives the scheduler either way; only the driving-potential contribution to
    /// the fine solver is gated by this flag, and that contribution ships off.
    ///
    /// `true`: `settle_tick` receives `&coarse_state.eta` / `&coarse_state.delta` whenever
    /// `coarse.available`, exactly as before this split existed.
    ///
    /// `false`: `settle_tick` is called with empty `coarse_eta`/`coarse_delta` slices, the exact
    /// same "not coupled" signal `coarse.available == false` already produces (see
    /// `physics::coarse_delta_eta`'s doc comment) -- so `settle_tick` takes the identical code
    /// path it took before this coupling existed. `coarse_state`'s own buffers (`eta`, `delta`,
    /// `m_mass`, ...) keep advancing regardless (see above), so flipping this back on resumes
    /// from a live, not frozen, state.
    pub coarse_pressure_coupling: bool,

    /// OVERCLOCKING.md: the multi-rate block-scheduler debug toggle. Independent of
    /// `coarse_pressure_coupling` above -- this consumes `|coarse_state.delta|` directly (never
    /// `phi`/`gravity_head`), so it can be A/B'd on its own build regardless of whether the
    /// driving-potential coupling is also on. Defaults `false`, like every other debug toggle
    /// except `coarse_pressure_coupling`'s old default.
    ///
    /// `true`: `update()` derives a per-block clock rate `block_clock_rate[b]` (EARLY-STOP.md: an
    /// arbitrary value in `[1/8, 8]`, not power-of-two quantised) from `|Delta[b]|` and staleness
    /// (HIERARCHICAL-PRESSURE.md §7b's "priority function based on amount of disagreement and
    /// last simulation time"), then (a) skips a block whose rate is below 1x from the LOD
    /// scheduler's budget-tier competition on ticks outside its own schedule (S2: MUST and STALE
    /// are never touched, so this can only ever defer a low-priority sweep, never suppress a real
    /// one), and (b) for a block whose rate is above 1x, runs up to `round(rate)` real sub-step
    /// repetitions of `settle_tick` this frame instead of one -- EARLY-STOP.md: `rate` is an upper
    /// BOUND now, not a mandate, so a block that reaches local equilibrium before its budgeted
    /// repetitions are used stops early (`force_overclocked_blocks_active`); S3 still forces every
    /// grid-neighbour of a genuinely-still-running forced block, so a boundary edge is evaluated
    /// regardless of which side happens to own it by grid index. Also repurposes
    /// `block_heat_texels()` to show clock rate instead of the recent-activity heat -- see that
    /// function's doc comment.
    ///
    /// `false`: `block_clock_rate` is held at `1.0` everywhere and none of the above runs --
    /// bit-identical to the tree before this toggle existed.
    pub overclocking_enabled: bool,

    /// EARLY-STOP.md: which rule turns a block's disagreement signal into a clock rate.
    ///
    /// `false` — the ABSOLUTE rule: `rate = clamp(signal / CLOCK_DELTA_REF_FRAC, min, max)`. Every
    /// block is judged against a fixed reference fraction, so the number of blocks asking for 8x
    /// is whatever the scene happens to produce, and the frame's cost is unbounded from below by
    /// anything except early stop.
    ///
    /// `true` — the RANK rule (the user's design): sort blocks by signal and hand out rates by
    /// POSITION, filling a fixed ladder whose band sizes are inversely proportional to the rate
    /// (`n_r ∝ 1/r`), so every band does the SAME total work and the whole frame's block-step
    /// count is a constant fraction of the block count regardless of scene. Under this rule
    /// underclocking is what pays for overclocking: the 8x band can only be 1/8 the size of the
    /// 1x band, and the fractional bands below 1x are what free the budget for it. "I want to be
    /// able to simulate at 8x ... maybe we just need to sort the differences and assign equal
    /// slices" (the user).
    ///
    /// Both rules respect `min_clock_rate`/`max_clock_rate`; the ladder is clipped to that range.
    pub rank_clock_rates: bool,

    /// EARLY-STOP.md: cap the GRADIENT of the rate field, not the rate. "maybe we need to align
    /// sub step counts nearby or don't let them be off more than 1" (the user) -- the 2:1 balance
    /// rule from adaptive mesh refinement, where neighbouring cells may differ by at most one
    /// refinement level.
    ///
    /// Enforced DOWNWARD, by pulling fast blocks down to `min(neighbour) + 1`, iterated to a
    /// fixed point -- never by raising slow ones, since "we can't force blocks to simulate more.
    /// we are already too slow". So a lone 8x block surrounded by 1x neighbours becomes 2x, while
    /// a wide contiguous fast region keeps its full rate: only a region big enough to ramp can
    /// reach the ceiling.
    ///
    /// Two things follow. Work goes DOWN, because the ceiling is only reachable where the
    /// scheduler wants a whole neighbourhood fast. And boundary stalls go down, because a seam
    /// costs one repetition of mismatch per adjacent pair instead of up to seven: with a gradient
    /// of 1, a block and its neighbour differ by at most one repetition of participation.
    ///
    /// Rates below 1x are left alone. They are skip PERIODS rather than repetition counts, they
    /// already run at most once per frame, and grading them would only ever raise them.
    pub grade_clock_rates: bool,

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

    /// EARLY-STOP.md: whether a block's clock rate GATES its participation in the extra
    /// repetitions, or merely adds to it.
    ///
    /// `false` (the behaviour through 0b8868c) — `force_overclocked_blocks_active` only ever
    /// ADDS: it raises the displacement of fast blocks and their neighbours so they are certain
    /// to run. Every other block is still admitted on its own merits by `settle_tick`'s ordinary
    /// MUST classification, on EVERY repetition, because a block that moved in the previous
    /// repetition is by definition above the MUST bar. So a rate of 1x does not mean "runs once
    /// per frame" -- it means "runs in every repetition, like everything else", and the rates buy
    /// extra work without ever redirecting any. Measured: with only 34 blocks at 8x, the extra
    /// repetitions still ran ~370 blocks each.
    ///
    /// `true` — repetitions after the first run ONLY the participation set: blocks whose rate
    /// still clears the repetition index, plus the S3 neighbours they force. Everything else has
    /// its displacement stashed and zeroed for the duration of that `settle_tick` call, then
    /// restored with `max()` so a block that RECEIVED mass while sitting out (S2) keeps the
    /// larger, live value rather than the stale snapshot. This is what makes the ladder a
    /// redistribution of a fixed work budget instead of a multiplier on it.
    pub rate_gated_reps: bool,

    /// LATERAL-COARSE-CORRECTION.md: the coarse-grid flow correction, default **OFF**.
    ///
    /// `false`: bit-identical to the tree before this existed -- the ledger is not even enabled,
    /// so `flux_edge_apply`/`try_move` pay one predictable branch and nothing else.
    ///
    /// `true`: after the frame's `settle_tick` repetitions have run, the mass the coarse level
    /// actually moved across each tile face is compared with the mass the fine level actually
    /// moved across the same face, and the DIFFERENCE is applied as a limited flux
    /// (`physics::apply_coarse_flow_correction`). It exists because the fine level's lateral
    /// transport is bounded by a local CFL condition that the coarse level, being a coarser grid,
    /// is not subject to -- see that function's doc comment for the full argument and
    /// `coarse_correction_damping` for why the coarse level's answer is not taken at face value.
    ///
    /// Requires the coarse level (`coarse.available`) and the shipped geometry where a block IS a
    /// coarse tile; a no-op otherwise.
    pub coarse_flow_correction: bool,

    /// LATERAL-COARSE-CORRECTION.md: whether the correction also boosts GRAVITY-ALIGNED
    /// conveyance, not just lateral. Default `true` ("both axes", the user's call) and
    /// deliberately NOT exposed in the Debug panel -- it exists so the diagnostics can separate
    /// the two effects, which matters because they do not point the same way on liquid: the
    /// vertical boost measurably speeds drainage, and a body of water that drains faster has less
    /// time to level, so on Water the vertical half works against the lateral half.
    pub coarse_correction_vertical: bool,

    /// LATERAL-COARSE-CORRECTION.md: under-relaxation on the coarse-grid correction, in `[0, 1]`.
    ///
    /// The coarse level is an approximation of the fine one -- a different grid, and with no model
    /// of repose at all -- so its answer is damped rather than applied whole. `1.0` means "trust
    /// the coarse level exactly"; `0.0` disables the correction entirely and is equivalent to
    /// `coarse_flow_correction: false` for physics purposes.
    ///
    /// **The default is a starting value, not a measured optimum.** The trade it controls: a high
    /// damping acts fast but fights the fine level harder, and the specific failure to watch for
    /// is the coarse level asking to flatten a granular pile below its angle of repose, the fine
    /// level restoring it next tick, and the flanks ringing. `diag_lateral_corr` sweeps this.
    pub coarse_correction_damping: f32,

    /// LATERAL-COARSE-CORRECTION.md: last frame's correction statistics, for the Debug panel and
    /// the diagnostics. Zeroed every tick the correction does not run, so a stale reading cannot
    /// linger on screen after the toggle goes off.
    pub last_frame_correction: physics::LateralCorrectionStats,

    /// Upper end of the clock-rate range `update_block_clock_rates` clamps to -- the runtime,
    /// UI-adjustable form of `CLOCK_RATE_MAX` (which remains the default and the hard ceiling a
    /// caller is expected to stay under). This is the single knob that sets how many
    /// `settle_tick` repetitions a frame can cost: `update()`'s `extra_reps` is
    /// `round(max rate over all blocks) - 1`, so a frame costs at most `round(max_clock_rate)`
    /// repetitions no matter how far ahead the coarse level says a block is. Lowering it trades
    /// settling rate for frame time directly, which is why it is exposed rather than tuned in
    /// code -- see EARLY-STOP.md for the sweep. Values below `1.0` are clamped up: a "max" under
    /// the neutral rate would mean no block may ever run at 1x, which is underclocking, not a
    /// ceiling, and `min_clock_rate` is the control for that.
    pub max_clock_rate: f32,

    /// Lower end of the same range -- the runtime form of `CLOCK_RATE_MIN`. Set to `1.0` to
    /// disable underclocking entirely (no block is ever asked to sit out a tick) while leaving
    /// overclocking untouched; that is the control run that says what underclocking is actually
    /// buying, since `apply_underclock_skip` can only defer a low-priority sweep, never cancel a
    /// MUST one, and most blocks below 1x in a typical scene were not going to run anyway.
    pub min_clock_rate: f32,

    /// OVERCLOCKING.md / EARLY-STOP.md: per-block clock rate, an arbitrary value in `[1/8, 8]`
    /// (not power-of-two quantised -- see `CLOCK_RATE_MIN`'s doc comment for why quantisation was
    /// dropped), one entry per LOD block -- same indexing as `active_blocks`/`last_displacements`
    /// (`cols * rows`, `block_size == grid/64`). `1.0` (the neutral/default rate) everywhere while
    /// `overclocking_enabled` is `false`. Recomputed fresh from this tick's signal every call to
    /// `update_block_clock_rates` (no hysteresis/step-limiting memory -- with early stop bounding
    /// a block's real repetitions at its own physical settle point, `rate` is a budget, so exactly
    /// how it moves tick to tick matters far less than it used to).
    pub block_clock_rate: Vec<f32>,

    /// EARLY-STOP.md: the ACTUAL number of per-block interior sweeps `update()`'s most recent
    /// call ran, summed over every repetition of this frame's rep loop -- i.e. `sum` over blocks
    /// of how many times each one was genuinely simulated (`active_blocks[b] !=
    /// BlockActivity::Inactive` after a `settle_tick` call), not the naive `block_count *
    /// round(rate)` a caller might otherwise assume. Early stop and underclock-skip both make the
    /// real number diverge from that naive one -- this field exists so a caller (the web UI's
    /// footer readout) can show the divergence rather than the budget. Compare against
    /// `active_blocks.len()` (the block count) for a "how many effective clock-cycles did the grid
    /// spend this frame" ratio: below 1x means underclocking is genuinely idling blocks, above 1x
    /// means overclocking's extra repetitions are outweighing early stop's savings.
    ///
    /// `0` on any tick where nothing ran at all (`!has_active` -- `update()`'s own gate). Not
    /// cumulative across ticks; overwritten fresh every call to `update()`.
    pub last_frame_block_steps: u32,

    /// EARLY-STOP.md: block-boundary edges that went UNEVALUATED this frame because the block
    /// that OWNS them sat out a repetition its neighbour ran.
    ///
    /// Edges belong to their lower-index cell (physics.rs), so across a vertical block boundary
    /// the LEFT block owns the shared edges and across a horizontal one the TOP block does. Under
    /// `rate_gated_reps` a suppressed owner means those edges are simply not evaluated that
    /// repetition: no mass moves across that seam, and material can pile against it. That is a
    /// STALL, not a leak -- nothing is lost, it just does not flow -- and it is the mechanism to
    /// suspect first for a visible seam or hole along block edges.
    ///
    /// Counted per repetition and summed over the frame, so it is comparable against
    /// `last_frame_block_steps`. Zero whenever gating is off, and it falls sharply under
    /// `grade_clock_rates` (66-73% measured), which is what makes it the number to watch when
    /// trading allocation shape against artifacts.
    pub last_frame_stalled_boundaries: u32,

    /// EARLY-STOP.md: per block, how many `settle_tick` sweeps it ACTUALLY ran in the last frame
    /// -- its executed sub-step count, summed over the repetition loop. `last_frame_block_steps`
    /// is this vector's sum.
    ///
    /// This is the honest counterpart to `block_clock_rate`, which is only a BUDGET: early stop
    /// lets a block stop short of its rate, `rate_gated_reps` keeps it out of repetitions it did
    /// not earn, S3 forcing drags it into ones it did not ask for, and grading caps what it could
    /// have wanted in the first place. Only this says what happened. It is what the block
    /// heat-map overlay draws while overclocking is on, so the picture on screen is executed work
    /// rather than the plan for it.
    pub last_frame_block_substeps: Vec<u32>,

    /// STEP3-ADAPTIVE-COARSE.md (incremental restriction): per-BLOCK "did this block's fine
    /// heights possibly change this tick" flags, filled by `settle_tick`'s `touched_out`
    /// parameter at the end of whichever tick most recently actually ran the fine solver. Read
    /// by the FOLLOWING tick's `coarse_state.tick(..., Some(&self.blocks_touched))` -- the
    /// ordering is deliberate: `coarse_state.tick()` runs at the top of `update()`, before this
    /// tick's own `settle_tick` call, so it is observing heights as they stood at the end of the
    /// PREVIOUS tick, which is exactly what the previous tick's `touched_out` describes.
    /// Block index and coarse tile index coincide (`block_size == grid/64 == COARSE_GRID`), so
    /// this is indexed identically to `coarse_state.a_mass` -- see `coarse::CoarseState::
    /// restrict_incremental`'s doc comment for the exactness argument.
    ///
    /// Starts empty (`Vec::new()`), which `restrict_incremental` treats as "cannot vouch for
    /// this, do a full rebuild" via a length mismatch -- correct for the first tick after
    /// construction or after `generate_shape_mask` rebuilds `coarse`/`coarse_state` (both clear
    /// this field for the same reason: a stale touched set from a different mask/shape is not
    /// safe to trust). When `has_active` is false this tick (settle_tick does not run at all),
    /// explicitly cleared to all-`false` rather than left stale, since nothing could have
    /// changed.
    pub blocks_touched: Vec<bool>,

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

    /// "Perfect simulation" debug toggle. Off by default (today's shipped, budget-limited
    /// scheduler behaviour). When on, `update` force-admits every non-trivial block (inside the
    /// shape mask AND holding material — see `PERFECT_SIM_MATERIAL_EPSILON`) into
    /// `settle_tick`'s unconditional MUST tier every tick, ignoring `budget_n` entirely. This
    /// exists so the adaptive scheduler's own approximation can be A/B'd against the ground
    /// truth: several visual artifacts (gaps, slabs, stalled material) trace back to blocks that
    /// lost the budget competition, and this toggle shows what the simulation looks like without
    /// that competition. It is deliberately expensive — that is the point, not a bug.
    pub perfect_simulation: bool,

    /// "Fresh pressure field" debug toggle. Off by default (today's shipped in-loop, order-
    /// dependent `column_depth` computation, unchanged). When on, `column_depth`
    /// (depth-integrated lateral pressure — see `physics::LATERAL_PRESSURE_SCALE`'s doc comment)
    /// is instead computed by a standalone, unconditional pass (`physics::recompute_column_depth`)
    /// run once per tick, before `settle_tick`'s phase loop, over the frozen pre-tick heightmap
    /// snapshot, with the old in-loop write disabled for that tick. This is `settle_tick`'s
    /// `fresh_pressure_field` parameter, threaded straight through — see its doc comment there for
    /// the full mechanics and the one metric (`test_liquid_flowing_liquid_does_not_stand_in_walls`'s
    /// voids@160) it has actually been measured against, which is not an improvement on that
    /// metric. This toggle exists to let the standalone pass's actual on-screen behaviour be
    /// judged directly (A/B'd against the current default) rather than only through that one
    /// scalar — it is experimental, not a settled replacement for the default.
    pub fresh_pressure_field: bool,

    /// Which quantity feeds the per-cell pressure heat-map overlay (`pressure_field_texels`).
    /// `false` (default): today's shipped `column_depth`, unchanged. `true`: the PERSISTENT
    /// hydraulic head field (`head_field` below; task #55 step 2, rebuilt as incremental
    /// propagation -- see `physics::task55_head_field`'s module doc comment), converted to a
    /// pressure-like quantity via `physics::task55_head_field::head_field_to_pressure` (`p =
    /// head(i) - z(i)`, the same datum `task55_head_spec::head_at`/`pressure_at` use) so it lands
    /// on the SAME scale `column_depth` already renders through -- see `pressure_field_texels`
    /// for the shared normalisation both sources go through.
    ///
    /// Plumbed exactly like `fresh_pressure_field` just above: a plain
    /// UI-facing debug toggle, carried through `set_grid_size`'s sim rebuild rather than reset,
    /// never read by `settle_tick` or anything else that advances the simulation. This is
    /// VISUALISATION ONLY -- flipping it changes what `pressure_field_texels` returns and nothing
    /// about how `heightmap`/`column_depth`/any other simulation state evolves.
    ///
    /// COST: `head_field_to_pressure` is a pure `O(cells)` read-and-convert over the already-
    /// maintained `head_field` buffer -- no relaxation happens inside `pressure_field_texels`
    /// itself any more (unlike the deleted per-call `compute_head_field`, which measured 11.4ms
    /// at w=512 in release). The relaxation that actually populates `head_field` happens inside
    /// `settle_tick`, which advances it whenever EITHER `head_field_transport` OR this field is
    /// set (see `settle_tick`'s own `head_field_needs_advance`) -- so setting THIS field alone,
    /// with transport left off, is now enough to give the overlay a live, relaxing buffer;
    /// `head_field_transport` no longer needs to be on for this overlay to show anything
    /// meaningful. With BOTH left off, `head_field` sits at whatever `reset()`/construction last
    /// zero-filled it to, and this overlay source reads as uniformly dark for the same reason
    /// `column_depth` would over an unsimulated sim -- an honest reflection of what the persistent
    /// buffer currently holds, not a bug -- see `head_field`'s own doc comment. Note this does NOT
    /// make the two toggles equivalent: setting only THIS field advances `head_field` but never
    /// routes it into the edge solvers, so `update`'s simulation output stays byte-identical
    /// regardless of this field's value (`pressure_heatmap_head_field_toggle.rs`).
    pub pressure_heatmap_head_field: bool,

    /// Whether the pressure heat-map overlay is actually being DRAWN. Purely a cost gate: the
    /// saturation-decile refresh below is the only thing that reads it, and that exists solely to
    /// produce the overlay's legend, so it must not run when nobody is looking at the overlay.
    /// Mirrors the wasm wrapper's own `pressure_heatmap_enabled`, which owns the user-facing
    /// switch; kept here too so the sim can gate its own work without the renderer having to
    /// remember to ask.
    pub pressure_heatmap_overlay: bool,

    /// Task #70: tension at a completely empty cell, in the same units as the fill term and as one
    /// row of gravity head. Zero (the default) reproduces the pre-tension behaviour exactly.
    ///
    /// The pressure law had compression above capacity and NOTHING below it, so a cell at 0.99 and
    /// a cell at 0.05 both reported zero pressure and neither resisted being drained. The only
    /// stable states were "pinned at the ceiling" and "scraped out", which is the bimodal
    /// population behind the visible checkerboard. This is the missing restoring force. See
    /// `physics::overfill_pressure_val`.
    pub underfill_tension: f32,

    /// Task #70: the fluid's bulk stiffness — how hard a column resists compressing under its own
    /// weight. This is the dial the UI exposes; `overfill_capacity` is derived from it by
    /// `overfill_ceiling_for` and is no longer independently settable.
    ///
    /// The two are the same physical quantity stated twice, and letting a user set both is a
    /// footgun. Measured at stiffness 5.0 on a 128-grid: a settled column wants fill 1.06 at the
    /// surface rising to 1.68 at the floor. A 1.90 ceiling accommodates that and the heat map
    /// shows a clean depth gradient with all ten decile bands populated (504/492/510/503/473/533/
    /// 496/500/497/523 cells). A 1.10 ceiling cannot: 5382 of 6318 cells pin to exactly 1.100,
    /// nine of the ten bands collapse, and the fluid is back to being packed against a wall.
    /// Softer fluid needs a higher ceiling, so the ceiling follows the dial.
    pub overfill_stiffness: f32,

    /// Decile boundaries (9 values, D1..D9) of per-cell SATURATION -- `height / capacity`, where
    /// 1.0 is exactly full and anything above is overfill -- taken over cells that hold material.
    /// Empty at construction and until the first refresh.
    ///
    /// These drive the overfill heat-map's colouring, which is a histogram equalisation rather
    /// than a fixed scale: each decile gets one tenth of the occupied cells, so the overlay always
    /// spends its full colour range on the distribution actually present instead of compressing
    /// everything into one band. That is the property that makes "how saturated are we" readable
    /// at a glance, and it is why the boundary VALUES have to be surfaced in the UI alongside it
    /// -- without the legend an equalised map tells you the shape of the distribution but not its
    /// magnitude, and magnitude is the whole question.
    pub saturation_deciles: Vec<f32>,

    /// "Drive transport from the head field" debug toggle (task #55 step 3). Off by default
    /// (today's shipped `column_depth`/`GRAVITY_HEAD_SCALE`-derived driving head, unchanged --
    /// bit-identical). When on, `settle_tick`'s lateral and vertical (gravity-aligned) edge
    /// solvers read their driving head from the persistent `head_field` buffer instead, for
    /// edges where BOTH endpoints are liquid (`liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD`).
    /// This is `settle_tick`'s `head_field_transport` parameter, threaded straight through, same
    /// shape as `fresh_pressure_field` above. Also (but not EXCLUSIVELY any more --
    /// see `pressure_heatmap_head_field`'s own doc comment) one of the two conditions that makes
    /// `head_field` advance at all this tick (`physics::task55_head_field::advance_head_field`,
    /// called once per tick from inside `settle_tick` when this OR `pressure_heatmap_head_field`
    /// is on).
    ///
    /// Scope: LIQUID ONLY, deliberately. The field has no yield criterion yet, so applying it to
    /// granular material would flatten a resting pile's angle of repose (a permanent surface
    /// gradient that must produce ZERO flow) -- a prior shipped attempt at #55 (the "fast liquid
    /// levelling" multigrid pass, since deleted) made exactly this mistake in a different way
    /// (moving HEIGHTS globally instead of driving the existing local flux solver) and was
    /// visually refuted: water moved too fast, falling water drifted sideways, and the
    /// surface stayed dead flat across actively draining necks. This toggle instead only replaces
    /// the DRIVING HEAD inside the existing, mass-conserving, per-edge flux solver -- the solver's
    /// own donor/acceptor clamps (`clamp_edge_feasible`) and per-edge momentum/damping (the
    /// velocity bound) are inherited unchanged, not reinvented.
    ///
    /// COST: `physics::task55_head_field::HEAD_FIELD_SWEEPS_PER_TICK` (2) local relaxation sweeps
    /// over the wet cells, `O(wet_cells)` and fixed regardless of grid resolution -- see that
    /// constant's own doc comment. This REPLACES the old design's per-call solve-to-convergence
    /// (measured 11.4ms at w=512 in release, and which did not actually converge at that scale --
    /// see `physics::task55_head_field`'s module doc comment), so the ongoing per-tick cost while
    /// this flag is on is now small and fixed rather than large and unbounded; still paid only
    /// while this flag (or the test gate) is on, never otherwise.
    pub head_field_transport: bool,

    /// "Pressure-sensitive flow rate" debug toggle (task #63). Off by default (today's shipped
    /// conveyance coefficient, independent of how much head a cell carries -- bit-identical).
    /// When on, a LIQUID-ONLY edge's conveyance coefficient (`physics::flux_edge_candidate`'s
    /// `c_sq`) is scaled by `sqrt(donor head / PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD)`, clamped at
    /// `1.0`, at both the vertical and lateral edge sites. So water 20 reference rows deep pushes
    /// at the full shipped rate, water 10 rows deep at 0.71 of it, and a one-row surface film at
    /// 0.22 -- a depth ORDERING, not just a thin-film cutoff.
    ///
    /// The square root is Torricelli (`v = sqrt(2*g*h)`), not a fitted curve; the reference depth
    /// is a stated design choice, because the top of the range is already at the CFL bound and so
    /// a depth ordering can only be produced by slowing the shallow end. See both constants' doc
    /// comments in `physics.rs`.
    ///
    /// SLOWS THE LOW END, never speeds the high end. The multiplier
    /// (`physics::pressure_rate_factor`) is capped at exactly `1.0` and is exactly `1.0` at and
    /// above the reference depth, so the deepest water in a scene is untouched and nothing can be
    /// pushed past the CFL bound `c_sq` was chosen to respect. Whatever this does to a scenario,
    /// it can only ever be a REDUCTION in flux relative to the toggle being off.
    ///
    /// FREE FALL IS EXEMPT, by construction rather than by a special case. Pressure comes from
    /// the head field, which pins unsupported material to `head = z` and therefore to exactly zero
    /// head; the rate law returns `1.0` there. A ballistic parcel has no contact pressure for a
    /// pressure-derived rate to be sensitive to, so it keeps falling at full speed. See
    /// `physics::task55_head_field::rows_of_head_at` for why the zero/positive separation is
    /// exact and not an epsilon.
    ///
    /// INDEPENDENT OF `head_field_transport` above, deliberately. This reads `head_field` for the
    /// donor's pressure but does not change which driving head any edge uses, so the two can be
    /// evaluated separately -- which matters while transport is still blocked on #64. It is a
    /// third condition (alongside `head_field_transport` and `pressure_heatmap_head_field`) that
    /// makes `head_field` advance at all this tick, so turning this on alone keeps the field live.
    ///
    /// Scope: LIQUID ONLY, same `LIQUID_ELLIPTIC_THRESHOLD` gate on both edge endpoints as
    /// `head_field_transport`, and for the same reason -- the head field has no yield criterion,
    /// so it must never reach granular material.
    pub pressure_sensitive_flow: bool,

    /// "Per-cell overfill pressure simulation" toggle (task #70).
    /// When on, cells can take on a small overfill (up to 1.50x nominal capacity) under
    /// overburden and transmit hydrostatic/Mohr-Coulomb pressure through stiffness gradient.
    pub overfill_pressure: bool,

    /// Maximum cell overfill capacity multiplier (e.g. 1.50 for 1.50x nominal capacity).
    pub overfill_capacity: f32,

    /// Per-block "how often was this block actually simulated" heat-map counter for the debug
    /// overlay, flattened row-major as `[block][bucket]`: `HEAT_NUM_BUCKETS` bytes per block
    /// (see that constant's doc comment). Length is always `active_blocks.len() *
    /// HEAT_NUM_BUCKETS` (kept in sync defensively in `update`, the same way `settle_tick`
    /// resizes its own per-block buffers).
    ///
    /// WHAT THIS MEASURES, EXACTLY: rather than a full 300-deep per-block ring buffer of
    /// simulated/not-simulated bits (1024 blocks * 300 bits = ~38KB, workable but wasteful for
    /// what is ultimately displayed as one blurry tint per block), the 300-tick window is
    /// divided into `HEAT_NUM_BUCKETS` chunks of `HEAT_BUCKET_TICKS` ticks each. Every tick, the
    /// current chunk's counter is incremented for each block `settle_tick` actually simulated;
    /// every `HEAT_BUCKET_TICKS` ticks, the chunk that is about to start representing the newest
    /// data is the same slot that held the OLDEST chunk (10 slots for a 10-chunk ring), so it is
    /// cleared right before reuse — that clear IS the "periodic decay". Summing all buckets and
    /// dividing by 300 (`block_heat_texels`/`block_heat_normalized`) gives the fraction of the
    /// trailing window the block was simulated in.
    ///
    /// HOW THIS DIFFERS FROM AN EXACT 300-TICK TRAILING COUNT: the count is exact at whole-chunk
    /// granularity but not at the tick level — a block simulated on tick 299-ago is
    /// indistinguishable from one simulated on tick 271-ago, since both just increment the same
    /// bucket. The aggregate also isn't pinned to exactly 300 ticks of history at every instant:
    /// it's 9 complete 30-tick chunks (270 ticks, exact) plus however many ticks have elapsed in
    /// the currently-filling 10th chunk (0..30 more), so the true window length breathes between
    /// 270 and 300 ticks rather than sitting fixed at 300. For a coarse heat tint this is well
    /// within what's visually distinguishable.
    pub block_heat_buckets: Vec<u8>,
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
    /// `block_size` (the LOD scheduler's block edge length in cells) scales with `grid_size`
    /// rather than staying an absolute constant, specifically `(grid_size / 64).max(1)`, so the
    /// grid is always tiled into the same 64x64 = 4096 blocks regardless of resolution. This
    /// keeps `budget_n` (and `BUDGET_MIN`/`BUDGET_STEP_*` in `update`) meaningful as the *same
    /// fraction* of the grid at every resolution, and it is load-bearing beyond that: the
    /// block-simulation heat-map overlay (`sandart-render`'s `HEAT_GRID_SIZE`,
    /// `update_block_heat`) uploads `block_heat_texels()` into a texture sized to a FIXED
    /// `HEAT_GRID_SIZE x HEAT_GRID_SIZE` with no bounds check on the source slice's length, so if
    /// the block count varied with resolution that upload would read out of bounds or corrupt the
    /// image at whichever resolution the block grid was smaller. Keeping `block_size` absolute
    /// instead of resolution-scaled would additionally have made low resolutions (e.g. 64/16 =
    /// 4x4 = 16 total blocks) fall entirely under `budget_n`'s minimum, disabling the LOD
    /// scheduler's throttling outright at low res and making 64 behave differently from 512 for
    /// scheduling reasons unrelated to physics.
    ///
    /// **Was `grid_size / 32` (32x32 = 1024 blocks); changed to `/ 64` so the LOD block is the
    /// same object as `coarse::CoarseGeometry`'s pressure tile** (`COARSE_GRID = 64` in
    /// `coarse.rs`, HIERARCHICAL-PRESSURE.md §2 "The LOD block and the pressure cell are the same
    /// object") — one restriction pass, one activity structure, once the coarse pressure level is
    /// wired in. Nothing reads `coarse.rs`'s output yet, so today this is purely a scheduling
    /// change: `budget_n` and the block-count constants below all had to move with it (see
    /// `update`'s `BUDGET_MIN`/`BUDGET_STEP_*`, and the two `budget_n = 1024` sites), and wake
    /// propagation (`activate_neighbor_upstream`/`_side` in `physics.rs`, which wake one adjacent
    /// *block*) now covers half as many cells per tick since a block is half as wide — see
    /// `artifacts/design/BLOCK-RESIZE.md` for the measurement.
    ///
    /// **The floor stays `.max(1)`, unchanged from before this change**, so grid 64 gets
    /// `block_size = 1` — the LOD scheduler degenerates to one block per cell there. This is a
    /// DIFFERENT decision from `coarse.rs`'s for the (currently unwired) pressure module, which
    /// disables itself below `t = 2` rather than floor: that module needs `t` (fine cells per
    /// coarse cell) to stay >= 2 so a coarse cell's own overfill pressure is never double-counted
    /// against itself. The LOD scheduler has no such correctness constraint — a 1-cell block is
    /// just the smallest possible scheduling unit, with no known wrong behaviour, only the loss of
    /// LOD grouping benefit at a resolution too small for that benefit to matter (4,096 cells
    /// total). Flooring at 2 instead was considered and rejected specifically because it would
    /// have broken the resolution-invariant block count described above: grid 64 would then be
    /// 32x32 = 1024 blocks while every other shipped resolution is 64x64 = 4096, and the heat-map
    /// texture upload above has no path for a smaller source buffer. Grid 128 (block_size 2, the
    /// smallest NON-degenerate case) is deliberately shipped without a floor even though
    /// `physics.rs` documents a slab artifact at `block_size = 2` elsewhere
    /// (`VERTICAL_PRESSURE_CAP_MULT`'s doc comment) — measured before shipping, see
    /// `artifacts/design/BLOCK-RESIZE.md`.
    pub fn new_with_size(grid_size: usize) -> Self {
        Self::new_with_block_divisor(grid_size, DEFAULT_BLOCK_DIVISOR)
    }

    /// `new_with_size`, with the LOD block edge length left open: `block_size = grid/divisor`.
    /// `DEFAULT_BLOCK_DIVISOR` (64) is the shipped geometry, where a block and a coarse tile are
    /// the same square. A SMALLER divisor means BIGGER blocks (32 -> 16-cell blocks at grid 512,
    /// 16 -> 32-cell blocks), which decouples the two: a block then covers several coarse tiles
    /// and the scheduler aggregates their disagreement (see `update_block_clock_rates`), which is
    /// the "each will have 4 disagreements to deal with" case. Exists so block size can be
    /// MEASURED rather than argued about -- see BLOCK-SIZE-SWEEP.md.
    pub fn new_with_block_divisor(grid_size: usize, divisor: usize) -> Self {
        let heightmap = generate_smooth_noise(12345u32, grid_size);
        let temp_heights = heightmap.data.clone();
        let sliding = vec![false; grid_size * grid_size];
        let edge_vel_h = vec![0.0f32; grid_size * grid_size];
        let edge_vel_v = vec![0.0f32; grid_size * grid_size];
        let column_depth = vec![0.0f32; grid_size * grid_size];
        let head_field = vec![0.0f32; grid_size * grid_size];
        let mut cell_colors = vec![0u8; grid_size * grid_size * 4];
        for chunk in cell_colors.chunks_exact_mut(4) {
            chunk[0] = 210;
            chunk[1] = 180;
            chunk[2] = 140;
            chunk[3] = 255;
        }
        let mut cell_props = vec![0.0f32; grid_size * grid_size * 4];
        // Initialize with default DrySand preset
        for chunk in cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = 0.00;
            chunk[PROP_THRESHOLD] = 0.08;
            chunk[PROP_FLOW_RATE] = 0.25;
            chunk[PROP_GRAIN_SIZE] = 0.45;
        }

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
        // 4x the pre-#(this task) value (256), matching the 4x block-count increase (1024 -> 4096
        // blocks at grid >= 128) so this stays the same *fraction* of the block grid it always
        // was. See `reset()` below for the other site, and `BUDGET_MIN`/`BUDGET_STEP_*` in
        // `update` for the rest of the throttle that had to move with it.
        let budget_n = 1024;
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
            head_field,
            seed: 98765u32,
            marble_radius: 0.018,
            material_mode: MaterialMode::default(),
            sandbox_shape: SandboxShape::default(),
            gravity_dir: Vec2::ZERO,
            neck_width: 0.005,
            hourglass_curve: 0.6,
            multistage_chambers: 8,
            shape_mask: vec![MASK_OUTSIDE; grid_size * grid_size],
            shape_mask_dirty: true,
            flipped: false,
            // Placeholder until `generate_shape_mask()` below does the real build -- shape_mask
            // is still all-MASK_OUTSIDE at this point in construction, so there is nothing
            // meaningful to build from yet.
            coarse: CoarseGeometry::empty(grid_size),
            coarse_state: CoarseState::new(COARSE_GRID),
            // Defaults ON -- see this field's own doc comment for why it differs from every
            // other debug toggle in the group, which default off.
            // Defaults OFF -- see this field's own doc comment for why (OVERCLOCKING.md split
            // it from the coarse level's own dynamics, which now run unconditionally).
            coarse_pressure_coupling: false,
            overclocking_enabled: false,
            rank_clock_rates: true,
            liquid_fall_jitter: 0.0,
            grade_clock_rates: true,
            rate_gated_reps: true,
            // LATERAL-COARSE-CORRECTION.md. Default OFF, like every other debug toggle in this
            // group. `COARSE_CORRECTION_DEFAULT_DAMPING` is a starting value, not a measured
            // optimum -- see its own doc comment.
            coarse_flow_correction: false,
            coarse_correction_vertical: true,
            coarse_correction_damping: COARSE_CORRECTION_DEFAULT_DAMPING,
            last_frame_correction: physics::LateralCorrectionStats::default(),
            max_clock_rate: CLOCK_RATE_MAX,
            min_clock_rate: CLOCK_RATE_MIN,
            block_clock_rate: vec![1.0f32; cols * rows],
            last_frame_block_steps: 0,
            last_frame_stalled_boundaries: 0,
            last_frame_block_substeps: vec![0u32; cols * rows],
            blocks_touched: Vec::new(),
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
            perfect_simulation: false,
            fresh_pressure_field: false,
            pressure_heatmap_head_field: false,
            pressure_heatmap_overlay: false,
            underfill_tension: 1.0,
            saturation_deciles: Vec::new(),
            head_field_transport: false,
            pressure_sensitive_flow: false,
            overfill_pressure: false,
            overfill_capacity: physics::overfill_ceiling_for(physics::OVERFILL_STIFFNESS_K),
            overfill_stiffness: physics::OVERFILL_STIFFNESS_K,
            block_heat_buckets: vec![0u8; cols * rows * HEAT_NUM_BUCKETS],
        };
        sim.generate_shape_mask();
        sim
    }

    /// Regenerate the shape mask from the current sandbox_shape, neck_width, hourglass_curve,
    /// and (for MultiStageHourglass) multistage_chambers. Call this whenever these parameters
    /// change. Sets shape_mask_dirty for GPU re-upload.
    pub fn generate_shape_mask(&mut self) {
        let w = self.heightmap.width;
        let h = self.heightmap.height;

        // Pass 1: Evaluate inside/safe for every cell using the existing physics evaluator
        for y in 0..h {
            let offset = y * w;
            for x in 0..w {
                let (inside, _safe) = physics::eval_sandbox_shape(
                    x, y, w, h,
                    self.sandbox_shape,
                    self.neck_width,
                    self.hourglass_curve,
                    self.multistage_chambers,
                    self.flipped,
                );
                self.shape_mask[offset + x] = if inside { MASK_INSIDE } else { MASK_OUTSIDE };
            }
        }

        // Pass 2: Mark boundary cells - any INSIDE cell with at least one OUTSIDE neighbor
        // We need a temporary copy to avoid read/write conflict
        let snapshot = self.shape_mask.clone();
        for y in 0..h {
            let offset = y * w;
            for x in 0..w {
                if snapshot[offset + x] == MASK_INSIDE {
                    let has_outside_neighbor =
                        (x == 0 || snapshot[offset + x - 1] == MASK_OUTSIDE) ||
                        (x + 1 >= w || snapshot[offset + x + 1] == MASK_OUTSIDE) ||
                        (y == 0 || snapshot[(y - 1) * w + x] == MASK_OUTSIDE) ||
                        (y + 1 >= h || snapshot[(y + 1) * w + x] == MASK_OUTSIDE);
                    if has_outside_neighbor {
                        self.shape_mask[offset + x] = MASK_BOUNDARY;
                    }
                }
            }
        }

        self.shape_mask_dirty = true;

        // Coarse pressure geometry (build step 1, HIERARCHICAL-PRESSURE.md §4/§9): rebuilt here,
        // and only here, so it is always exactly as fresh as shape_mask -- never a separate call
        // site to remember, never a separate staleness window.
        self.coarse = coarse::CoarseGeometry::build(&self.shape_mask, &self.cell_props, w);
        self.coarse_state = coarse::CoarseState::new(self.coarse.coarse_n);
        // A touched set from before this rebuild refers to a possibly different mask/block
        // layout; clearing forces the next `restrict_incremental` call to fall back to a full
        // rebuild (length mismatch), which is always correct.
        self.blocks_touched.clear();
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
                | SandboxShape::MultiStageHourglass
                | SandboxShape::GaltonBoard
                | SandboxShape::StaircaseCascade
                | SandboxShape::ProceduralFunnel
                | SandboxShape::MultiNeckHourglass
                | SandboxShape::UTubeFlowThrough
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
        self.head_field.fill(0.0);
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
        // Simulation state, like the three buffers just above -- not a UI setting, so a reset
        // clears it back to the neutral rate rather than carrying over whatever the scheduler
        // had converged to before. `overclocking_enabled` itself (the debug toggle) is untouched.
        self.block_clock_rate.fill(1.0);
        self.last_frame_block_steps = 0;
        self.last_frame_stalled_boundaries = 0;
        self.last_frame_block_substeps.iter_mut().for_each(|v| *v = 0);
        // Keep in sync with `new_with_size`'s `budget_n` initialisation above.
        self.budget_n = 1024;
        self.ema_frame_ms = 33.3;
        self.tick_count = 0;
        self.block_heat_buckets.fill(0);
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

        // Fills exactly tier 0 (the widest tier) of the MultiStageHourglass merging cascade:
        // `total_half = 0.42h` split evenly across however many tiers
        // `physics::multistage_tier_chambers(self.multistage_chambers)` produces, and this is
        // the bottom boundary of tier 0 (`-total_half + tier_h`). Must match the tier math in
        // `eval_sandbox_shape`'s `MultiStageHourglass` branch -- computed once here (not per
        // cell) since it only depends on `h` and the chamber count, not on `(x, y)`. At the
        // shipped default (multistage_chambers = 8, 4 tiers of `0.21h` each) this is exactly
        // `-0.21 * h`, today's original hard-coded value.
        let multistage_fill_threshold = {
            let total_half = 0.42 * h as f32;
            let n_tiers = physics::multistage_tier_chambers(self.multistage_chambers).len();
            let tier_h = (2.0 * total_half) / n_tiers as f32;
            -total_half + tier_h
        };

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

                let fill_threshold = if self.sandbox_shape == SandboxShape::MultiStageHourglass {
                    multistage_fill_threshold
                } else if self.sandbox_shape == SandboxShape::StaircaseCascade {
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

                for ch in 0..4 {
                    self.cell_colors.swap(i1 * 4 + ch, i2 * 4 + ch);
                }
                for ch in 0..4 {
                    self.cell_props.swap(i1 * 4 + ch, i2 * 4 + ch);
                }
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
        self.head_field.fill(0.0);

        // Turn the *structure* over too, not just what is in it. Symmetric shapes are unaffected
        // by construction; the asymmetric ones (StaircaseCascade's alternating shelves, the
        // MultiStageHourglass cascade's tiered chambers, ProceduralFunnel's noise) used to stay upright while
        // their contents mirrored into them.
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

    /// OVERCLOCKING.md / EARLY-STOP.md (HIERARCHICAL-PRESSURE.md §7b): update the per-block clock
    /// rate from coarse-fine disagreement and staleness -- the user's own words, "a priority
    /// function based on amount of disagreement and last simulation time". Block index and coarse
    /// tile index coincide (`block_size == grid/64 == COARSE_GRID`), so `coarse_state.delta[b]`
    /// and `coarse.capacity[b]` are read directly at index `b`.
    ///
    /// Two mappings from that signal onto a rate, selected by `rank_clock_rates` (see its doc
    /// comment for which is which and why the rank rule exists):
    ///
    /// - ABSOLUTE: `rate = clamp(signal / CLOCK_DELTA_REF_FRAC, min, max)`, continuous and
    ///   memoryless. No power-of-two quantisation, no octave-stepping, no hysteresis --
    ///   `CLOCK_RATE_MIN`'s doc comment has the correctness argument for why removing
    ///   quantisation is safe here. `signal == CLOCK_DELTA_REF_FRAC` maps to exactly 1x.
    /// - RANK: sort by signal, fill `CLOCK_RATE_LADDER` with band sizes `∝ 1/r`. The rate a block
    ///   gets depends on its POSITION among its peers, not on an absolute threshold, so the
    ///   frame's total block-step count is scene-independent.
    ///
    /// Staleness only ever pushes the signal UP (`1.0 + staleness * CLOCK_STALENESS_WEIGHT`,
    /// never a divisor) -- "underclock conservatively" (§7b): a block that has not run in a while
    /// is treated as more urgent to run, never less, since missed transport is lost, not deferred
    /// (the `+/-1.0` clamp in `flux_edge_candidate` means a block cannot catch up on waking).
    ///
    /// Does nothing (leaves `block_clock_rate` at whatever it was) if the coarse level's buffers
    /// are not sized to match -- the caller's `overclocking_enabled && coarse.available` guard is
    /// the normal reason this would be skipped, this is a defensive fallback.
    fn update_block_clock_rates(&mut self) {
        let n = self.block_clock_rate.len();
        let tiles = self.coarse_state.delta.len();
        if self.coarse.capacity.len() != tiles || self.last_simulated_ticks.len() != n || tiles == 0
        {
            return;
        }
        // Block and coarse tile are the same square only at the default block divisor. When
        // blocks are bigger, a block covers several tiles and its disagreement is the MAX over
        // them, not the sum: the signal is "how far out of agreement is this region", and one
        // badly-out-of-agreement tile inside a block is a reason to run the whole block, while
        // summing would let a big block's many quiet tiles dilute it. Cheap because it is one
        // pass over tiles, not per block per tile.
        let cols = self.block_size_cols();
        let tile_n = COARSE_GRID;
        let mut block_delta_frac = vec![0.0f32; n];
        if tiles == n {
            for b in 0..n {
                let cap = self.coarse.capacity[b].max(1e-6);
                block_delta_frac[b] = (self.coarse_state.delta[b].abs() / cap).max(0.0);
            }
        } else {
            let g = self.heightmap.width;
            let t = (g / tile_n).max(1);
            let bs = self.block_size.max(1);
            if cols == 0 {
                return;
            }
            // Driven from the BLOCK side so both directions work: a block bigger than a tile takes
            // the max over the tiles it covers, a block smaller than a tile takes the one tile it
            // sits inside (its siblings inside that tile all read the same value, which is the
            // honest answer -- the coarse level has no finer opinion to give).
            for b in 0..n {
                let bx = b % cols;
                let by = b / cols;
                let cx0 = (bx * bs) / t;
                let cx1 = (((bx + 1) * bs - 1) / t).min(tile_n - 1);
                let cy0 = (by * bs) / t;
                let cy1 = (((by + 1) * bs - 1) / t).min(tile_n - 1);
                let mut best = 0.0f32;
                for cy in cy0..=cy1.max(cy0) {
                    for cx in cx0..=cx1.max(cx0) {
                        let c = cy * tile_n + cx;
                        if c >= tiles {
                            continue;
                        }
                        let cap = self.coarse.capacity[c].max(1e-6);
                        let frac = (self.coarse_state.delta[c].abs() / cap).max(0.0);
                        if frac > best {
                            best = frac;
                        }
                    }
                }
                block_delta_frac[b] = best;
            }
        }
        // Hoisted: the range is a per-frame setting, not per-block. `hi` is floored at 1.0 so a
        // slider dragged to its bottom end means "no overclocking", never "every block
        // underclocked"; `lo` is then held at or below `hi` so the clamp can never invert.
        let hi = self.max_clock_rate.clamp(1.0, CLOCK_RATE_MAX);
        let lo = self.min_clock_rate.clamp(CLOCK_RATE_MIN, hi);

        // The signal itself is the same under both rules -- only its mapping onto a rate differs.
        let mut signal = vec![0.0f32; n];
        for b in 0..n {
            let staleness = self.tick_count.wrapping_sub(self.last_simulated_ticks[b]).min(1000) as f32;
            signal[b] = block_delta_frac[b] * (1.0 + staleness * CLOCK_STALENESS_WEIGHT);
        }

        if !self.rank_clock_rates {
            for b in 0..n {
                // The ABSOLUTE rule: continuous, memoryless, clamped. Reads only `signal`, never
                // the previous rate -- no octave index to step, no hysteresis band, because
                // `rate` is a budget early stop is free to underspend.
                self.block_clock_rate[b] = (signal[b] / CLOCK_DELTA_REF_FRAC).clamp(lo, hi);
            }
            self.grade_block_clock_rates();
            return;
        }

        // The RANK rule. A block with no disagreement at all is not ranked -- it goes straight to
        // the bottom rate. Without this an empty or fully-settled scene would still hand its
        // top 1/127th of blocks an 8x rate purely on floating-point dust, which is exactly the
        // "spend the budget on blocks that do not need it" failure the rank rule exists to avoid.
        self.block_clock_rate.iter_mut().for_each(|r| *r = lo);
        let mut order: Vec<u32> = (0..n as u32).filter(|&b| signal[b as usize] > 0.0).collect();
        if order.is_empty() {
            return;
        }
        // Descending by signal. `partial_cmp` cannot see a NaN here (`delta` is finite and
        // `capacity` is floored at 1e-6) but is handled rather than unwrapped, since a NaN would
        // otherwise panic the whole frame over a scheduling hint.
        order.sort_unstable_by(|&a, &b| {
            signal[b as usize]
                .partial_cmp(&signal[a as usize])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Ladder clipped to the live slider range. `hi >= 1.0` always, so 1x always survives and
        // the ladder is never empty.
        let ladder: Vec<f32> = CLOCK_RATE_LADDER
            .iter()
            .cloned()
            .filter(|&r| r <= hi + 1e-6 && r >= lo - 1e-6)
            .collect();
        // Band sizes `n_r ∝ 1/r`: equal WORK per band, so the 8x band is an eighth of the 1x
        // band and the fractional bands fund the fast ones. A gentler `1/lg(1+r)` falloff was
        // tried and removed once grading landed -- grading already produces the wide, contiguous
        // fast regions that falloff was widening the bands to get, and it does it by looking at
        // the scene instead of by handing out more budget everywhere.
        let inv_sum: f32 = ladder.iter().map(|&r| 1.0 / r).sum();
        let m = order.len();
        let mut start = 0usize;
        for (i, &rate) in ladder.iter().enumerate() {
            // `n_r ∝ 1/r`: equal work per band. The last band absorbs the rounding remainder so
            // every ranked block is assigned exactly once.
            let count = if i + 1 == ladder.len() {
                m - start
            } else {
                ((m as f32) * (1.0 / rate) / inv_sum).round() as usize
            };
            let end = (start + count).min(m);
            for &b in &order[start..end] {
                self.block_clock_rate[b as usize] = rate;
            }
            start = end;
            if start >= m {
                break;
            }
        }
        self.grade_block_clock_rates();
    }

    /// See `grade_clock_rates`. Pulls every block down to at most one repetition above its
    /// slowest grid neighbour, iterated until nothing changes.
    ///
    /// Works in REPETITION space (`round(rate)`), not rate space, because that is what a
    /// boundary stall is counted in: block `b` participates in repetition `r` iff
    /// `round(rate) > r`, so two neighbours differing by one repetition can mismatch on at most
    /// one repetition. Sub-1x rates are floored at 1 for the comparison only and written back
    /// untouched -- they are skip periods, not repetition counts.
    ///
    /// Terminates: every pass either changes nothing or lowers at least one block by at least
    /// one whole repetition, and repetitions are bounded below by 1, so the loop is bounded by
    /// `CLOCK_RATE_MAX` passes. The explicit cap is belt-and-braces against a future
    /// non-monotone edit.
    fn grade_block_clock_rates(&mut self) {
        if !self.grade_clock_rates {
            return;
        }
        let n = self.block_clock_rate.len();
        let cols = self.block_size_cols();
        if cols == 0 || n == 0 || n % cols != 0 {
            return;
        }
        let rows = n / cols;
        // Repetition count per block; sub-1x blocks read as 1 (they run at most once).
        let mut reps: Vec<u32> = self
            .block_clock_rate
            .iter()
            .map(|&r| if r < 1.0 { 1 } else { r.round().max(1.0) as u32 })
            .collect();
        for _ in 0..(CLOCK_RATE_MAX as u32 + 1) {
            let mut changed = false;
            for b in 0..n {
                if reps[b] <= 1 {
                    continue;
                }
                let bx = b % cols;
                let by = b / cols;
                let mut min_nb = u32::MAX;
                if bx > 0 { min_nb = min_nb.min(reps[b - 1]); }
                if bx + 1 < cols { min_nb = min_nb.min(reps[b + 1]); }
                if by > 0 { min_nb = min_nb.min(reps[b - cols]); }
                if by + 1 < rows { min_nb = min_nb.min(reps[b + cols]); }
                if min_nb == u32::MAX {
                    continue;
                }
                let capped = reps[b].min(min_nb.saturating_add(1));
                if capped < reps[b] {
                    reps[b] = capped;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for b in 0..n {
            // Only ever written DOWN, and only for blocks that were overclocked to begin with:
            // a sub-1x rate is left exactly as the allocator set it.
            if self.block_clock_rate[b] >= 1.0 {
                self.block_clock_rate[b] = self.block_clock_rate[b].min(reps[b] as f32);
            }
        }
    }

    /// S2 (HIERARCHICAL-PRESSURE.md §7b: "underclocked means does not sweep its interior, NOT
    /// frozen"). For a block whose rate is below 1x AND is not currently MUST-worthy
    /// (`last_displacements[b] < MUST_SIMULATE_THRESHOLD` -- so this never touches a block with
    /// real work to do), zero its recorded displacement on ticks outside its own schedule so
    /// `settle_tick`'s classification loop (physics.rs) does not admit it into the budget-tier
    /// `rest_candidates` this tick. MUST and STALE are never touched here -- the staleness floor
    /// (`MAX_STALENESS` in physics.rs) stays the independent backstop the design requires, and
    /// the existing `activate_neighbor` machinery (untouched) still lets a skipped block receive
    /// mass from a running neighbour and be copied back. This is deliberately the ONLY mechanism
    /// by which underclocking has any effect: it can defer a low-priority sweep, it can never
    /// cancel a real one.
    ///
    /// `phase = b % period` spreads underclocked blocks' scheduled ticks across the period
    /// instead of every block at a given rate waking on the same tick.
    fn apply_underclock_skip(&mut self) {
        let n = self.block_clock_rate.len().min(self.last_displacements.len());
        for b in 0..n {
            let rate = self.block_clock_rate[b];
            if rate >= 1.0 {
                continue;
            }
            if self.last_displacements[b] >= physics::MUST_SIMULATE_THRESHOLD {
                continue;
            }
            let period = (1.0 / rate).round().max(1.0) as u32;
            let phase = (b as u32) % period;
            if self.tick_count % period != phase {
                self.last_displacements[b] = 0.0;
            }
        }
    }

    /// S3 (HIERARCHICAL-PRESSURE.md §7b: "edge ownership must follow the FASTER block" -- "the
    /// most likely place for a multi-rate scheme to lose mass or stall a front"). `update()`
    /// calls `settle_tick` again within the SAME rendered frame for a block's extra sub-step
    /// repetitions. Edges are owned by their lower-index cell (see physics.rs), so on a
    /// repetition where a fast block runs but its slower neighbour does not, a boundary edge
    /// whose owner happens to be that slower (non-running) neighbour would silently never be
    /// evaluated this repetition -- chosen by grid geometry, not physics.
    ///
    /// Rather than tracking which side owns which specific edge, this forces EVERY grid-adjacent
    /// neighbour of a block that is genuinely overclocked (`rate > 1`) and STILL RUNNING this
    /// repetition to also run this repetition, regardless of the neighbour's own rate, its own
    /// settled state, or which side owns the shared edge. This does strictly more work than the
    /// minimum fix (a forced neighbour runs whether or not it actually owns the boundary edge) --
    /// the simple, conservative choice over a surgical per-edge one, matching "simplicity is now a
    /// feature".
    ///
    /// EARLY-STOP.md: `rate` is a clock BUDGET, not a mandate. A block's own eligibility for this
    /// repetition (the `seed` set below) now additionally requires that its last real,
    /// physically-computed `last_displacements` -- written by the previous repetition's
    /// `settle_tick` call, read here BEFORE this call floors it again -- is still at or above
    /// `MUST_SIMULATE_THRESHOLD`. A block that already reached local equilibrium stops being
    /// re-forced into its own interior sweep for the rest of this frame's budget; PERF-PROFILE.md
    /// §3 measured ~59-62% of a rate>1 block's extra repetitions running on blocks that had
    /// already settled by the first one, which is exactly the waste this removes. This reads
    /// live, not cached, state, so S2 still holds: if a later repetition's neighbour-forcing (just
    /// below) or the ordinary flux/`activate_neighbor` machinery pushes real mass into a
    /// "settled" block, its `last_displacements` rises back above threshold and it becomes
    /// eligible again on the next repetition -- a settled block can wake, it just does not run
    /// for free while idle.
    ///
    /// Neighbour-forcing (S3) itself is UNCHANGED by early stop and must not be gated on the
    /// neighbour's own settled state: an edge the neighbour owns still has to be evaluated on
    /// every repetition the fast block genuinely runs, whether or not the neighbour's interior has
    /// anything left to do -- that neighbour's own interior sweep is cheap relative to silently
    /// losing mass across an unevaluated edge.
    ///
    /// `rep` is the repetition index (0-based) within this frame; only blocks whose rate clears
    /// `rep` (i.e. still have iterations left) AND have not yet settled are seeded.
    fn force_overclocked_blocks_active(&mut self, rep: u32) -> Vec<bool> {
        let n = self.block_clock_rate.len();
        let cols = self.block_size_cols();
        let rows = if cols > 0 { n / cols } else { 0 };
        if cols == 0 || rows == 0 || cols * rows != n {
            // An all-true mask, not an empty one: the caller uses this to decide who may SIT OUT,
            // and a degenerate block grid must not be read as "nobody may run".
            return vec![true; n];
        }
        let mut forced = vec![false; n];
        // Blocks that are nominally scheduled this repetition, whether or not they still have work
        // of their own -- these are what force neighbours (S3). See the comment below.
        let mut seed = vec![false; n];
        for b in 0..n {
            let rate = self.block_clock_rate[b];
            // EARLY-STOP: `rate` is a BUDGET, not a mandate. A block whose own
            // physically-computed displacement -- written by the PREVIOUS repetition's
            // `settle_tick`, before this function overwrites it below -- has fallen under the
            // scheduler's own settle bar has reached local equilibrium and gains nothing from
            // further repetitions. The profile measured ~59-62% of extra sub-step executions
            // running on exactly such blocks. This is eligibility only: a settled block still
            // RECEIVES mass from running neighbours (S2), and neighbour-forcing (S3) below is
            // deliberately NOT gated on it, so a clock-domain boundary never runs at the slow
            // side's rate. A block that a neighbour pushes into has its displacement rise back
            // above the bar and becomes eligible again on the next repetition.
            // MASS-ERR-DIAGNOSIS.md: `still_has_work` gates whether a block sweeps its OWN
            // interior. It must NOT also gate whether the block keeps acting as a neighbour-FORCER,
            // and conflating the two was a real S3 violation with a measured spatial signature:
            // a fast block at a clock-domain boundary that settled mid-frame stopped force-waking
            // its slower neighbour, and that neighbour -- already zeroed by `apply_underclock_skip`
            // on ticks outside its own schedule -- then had no route back into `will_simulate` for
            // the rest of the frame, so its owned edge into the fast block went unevaluated.
            // Measured as a REDISTRIBUTION (per-block excess summed to zero within 0.2%) localised
            // to two bands one block-row apart with opposite signs and matched magnitudes.
            //
            // So: `scheduled` (nominal budget only) decides forcing; `scheduled && still_has_work`
            // decides running. Early stop keeps its saving on interiors, S3 keeps its guarantee.
            let scheduled = rate > 1.0 && (rate.round() as u32) > rep;
            let still_has_work = self.last_displacements[b] >= physics::MUST_SIMULATE_THRESHOLD;
            if scheduled && still_has_work {
                forced[b] = true;
            }
            if scheduled {
                seed[b] = true;
            }
        }
        for b in 0..n {
            if !seed[b] {
                continue;
            }
            let bx = b % cols;
            let by = b / cols;
            if bx > 0 { forced[b - 1] = true; }
            if bx + 1 < cols { forced[b + 1] = true; }
            if by > 0 { forced[b - cols] = true; }
            if by + 1 < rows { forced[b + cols] = true; }
        }
        for b in 0..n.min(self.last_displacements.len()) {
            if forced[b] {
                self.last_displacements[b] =
                    self.last_displacements[b].max(physics::MUST_SIMULATE_THRESHOLD * 2.0);
            }
        }
        forced
    }

    /// The block grid's column count -- `(grid_width + block_size - 1) / block_size`, the same
    /// arithmetic `update()`/`settle_tick` use, exposed here so the clock-rate helpers above can
    /// map a flat block index back to `(bx, by)` without re-deriving it inline three times.
    /// Map a per-BLOCK touched mask onto the per-TILE one `CoarseState::restrict_incremental`
    /// expects. Returns the input unchanged (cloned) when the two grids already coincide, which
    /// is the shipped case. A tile inherits the flag of the block containing its top-left cell;
    /// when blocks are bigger than tiles this over-reports (every tile in a touched block is
    /// marked), which costs re-aggregation work and can never miss a change -- the direction that
    /// keeps `A[C]` correct.
    fn expand_touched_to_tiles(
        touched: &[bool],
        grid: usize,
        block_size: usize,
    ) -> Option<Vec<bool>> {
        let tile_n = COARSE_GRID;
        let n = tile_n * tile_n;
        // `None` means "the caller's own block mask is already tile-indexed, use it directly" --
        // the shipped geometry, and the reason this costs nothing there.
        if touched.len() == n || touched.is_empty() || block_size == 0 || grid == 0 {
            return None;
        }
        let cols = (grid + block_size - 1) / block_size;
        let rows = (touched.len() + cols.max(1) - 1) / cols.max(1);
        let t = (grid / tile_n).max(1);
        // Driven from the BLOCK side, ORing into every tile a touched block overlaps, so this is
        // correct whether blocks are bigger than tiles (one block marks many) or smaller (many
        // blocks share one tile and any of them marks it). Over-reporting is the safe direction:
        // it costs re-aggregation, it cannot miss a change.
        let mut out = vec![false; n];
        for b in 0..touched.len() {
            if !touched[b] {
                continue;
            }
            let bx = b % cols;
            let by = b / cols;
            if by >= rows {
                continue;
            }
            let cx0 = (bx * block_size) / t;
            let cx1 = (((bx + 1) * block_size - 1) / t).min(tile_n - 1);
            let cy0 = (by * block_size) / t;
            let cy1 = (((by + 1) * block_size - 1) / t).min(tile_n - 1);
            for cy in cy0..=cy1.max(cy0) {
                for cx in cx0..=cx1.max(cx0) {
                    out[cy * tile_n + cx] = true;
                }
            }
        }
        Some(out)
    }

    fn block_size_cols(&self) -> usize {
        let w = self.heightmap.width;
        if self.block_size == 0 { 0 } else { (w + self.block_size - 1) / self.block_size }
    }

    /// Row-major block-heat texel bytes for the heat-map debug overlay, one byte per block
    /// (always a 32x32 grid — see `new_with_size`'s doc comment), ready for direct upload as an
    /// R8Unorm GPU texture: `byte = round((times_simulated_in_window / 300) * 255)`, clamped.
    /// See `block_heat_buckets` for exactly what "times simulated in window" means and the
    /// approximation it makes versus a true 300-tick trailing count.
    ///
    /// OVERCLOCKING.md: while `overclocking_enabled` is on, this is repurposed -- same texture,
    /// same upload path, no shader or pipeline changes -- to show per-block SUB-STEPS ACTUALLY
    /// EXECUTED in the last frame (`last_frame_block_substeps`), not the clock rate it was
    /// budgeted.
    ///
    /// It drew the planned rate until now. The two are very different pictures once early stop,
    /// `rate_gated_reps`, S3 forcing and grading are all in play: a block can be budgeted 8x and
    /// run twice, or be rated 1x and run four times because a fast neighbour kept forcing it. The
    /// overlay exists to answer "where is the time going", and only the executed count answers
    /// that.
    ///
    /// Byte value is `log2(1 + substeps) / log2(1 + ceiling)` mapped onto `[0, 255]`, with the
    /// ceiling taken from `max_clock_rate` so the scale follows the slider rather than a constant:
    /// a block that ran zero times is black, one sweep (the ordinary, unclocked amount) sits low
    /// on the ramp, and only a block spending the whole budget reads hot. Log rather than linear
    /// because the interesting range is 1-4 sweeps and a linear ramp against a ceiling of 16
    /// would leave all of it in the bottom quarter.
    pub fn block_heat_texels(&self) -> Vec<u8> {
        if self.overclocking_enabled
            && self.last_frame_block_substeps.len() == self.active_blocks.len()
        {
            let ceiling = self.max_clock_rate.clamp(1.0, CLOCK_RATE_MAX);
            let denom = (1.0 + ceiling).log2().max(1e-6);
            return self
                .last_frame_block_substeps
                .iter()
                .map(|&s| {
                    let v = (1.0 + s as f32).log2() / denom;
                    (v.clamp(0.0, 1.0) * 255.0).round() as u8
                })
                .collect();
        }
        let num_blocks = self.block_heat_buckets.len() / HEAT_NUM_BUCKETS;
        (0..num_blocks)
            .map(|b| {
                let sum: u32 = (0..HEAT_NUM_BUCKETS)
                    .map(|k| self.block_heat_buckets[b * HEAT_NUM_BUCKETS + k] as u32)
                    .sum();
                ((sum.min(300) as f32 / 300.0) * 255.0).round() as u8
            })
            .collect()
    }

    /// Row-major per-CELL pressure-field heat-map texels, one byte per grid cell (`grid_size *
    /// grid_size` — NOT the fixed 32x32 block grid `block_heat_texels` above uses), ready for
    /// direct upload as an R8Unorm GPU texture. Source quantity depends on
    /// `pressure_heatmap_head_field`:
    ///
    /// - `false` (default): `column_depth` directly (see that field's doc comment for what it
    ///   measures — the hydrostatic overburden driving lateral, and now vertical, flow), so the
    ///   overlay tracks whichever pass currently populates it: both the default in-loop
    ///   computation and the `fresh_pressure_field` standalone pass write the same `column_depth`
    ///   array, so flipping THAT toggle is automatically reflected here with no extra plumbing.
    /// - `true`: `physics::task55_head_field::head_field_to_pressure`, reading the PERSISTENT
    ///   `head_field` buffer (task #55 step 2, rebuilt as incremental propagation — see
    ///   `physics::task55_head_field`'s module doc comment) and converting it from an elevation to
    ///   a pressure-like quantity (`p = head(i) - z(i)`) so it lands on the exact same scale as
    ///   `column_depth` below. NOT computed fresh here — `head_field` is simulation state
    ///   maintained by `settle_tick` while `head_field_transport` is on (see that field's own doc
    ///   comment), so this is a cheap `O(cells)` read-and-convert, not a solve.
    ///
    /// SCALING (the design decision that makes this overlay legible at all): both sources are
    /// unbounded and span orders of magnitude in practice — 0 in voids, through roughly
    /// 24/64/104/144 down a resting sand column, to 464 for water 60 rows deep. A naive linear
    /// map against a fixed max would render as a nearly-black screen with a bright sliver at the
    /// very bottom, revealing no structure at all.
    ///
    /// This instead uses `ln(1 + p) / ln(1 + PRESSURE_HEATMAP_LOG_MAX)`, clamped to [0, 1]. Log
    /// compression is the right shape here because low pressure is the interesting end (voids,
    /// shallow columns, the free surface, an unsupported free-falling body) — the derivative of
    /// `ln(1 + x)` is `1 / (1 + x)`, steepest near zero and flattening out at the high end, so
    /// this gives maximum visual contrast to exactly the low-pressure structure a reader is
    /// trying to resolve (e.g. the ~55-cell void this overlay was built to diagnose), while still
    /// spreading the high end (deep water) across a visibly distinct hot range rather than
    /// clipping it all to one color. Applying the SAME map to both sources (rather than each
    /// having its own scale) is what makes an A/B between them meaningful: a roofed cave or the
    /// U-tube's basin should read near-dark under `column_depth` (little material directly
    /// overhead) and bright under the head field (it knows about the connected column beside it)
    /// on ONE fixed colour scale, not two incomparable ones. See `PRESSURE_HEATMAP_LOG_MAX`'s own
    /// doc comment for why the denominator is a fixed constant rather than each frame's own max.
    pub fn pressure_field_texels(&self) -> Vec<u8> {
        let log_max = (1.0 + PRESSURE_HEATMAP_LOG_MAX).ln();
        let to_byte = |depth: f32| -> u8 {
            let normalized = (1.0 + depth.max(0.0)).ln() / log_max;
            (normalized.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        if self.overfill_pressure {
            // SATURATION DECILES, not a log-compressed pressure. The question this overlay is
            // asked under the overfill model is "how saturated are we" -- how close the material
            // is to capacity, and where it has gone past. A fixed scale answers that badly:
            // saturation lives almost entirely in a narrow band around 1.0, so any fixed map
            // renders the whole body one flat colour and hides exactly the structure being looked
            // for. Equalising against the frame's own decile boundaries spends the full colour
            // range on the distribution actually present.
            //
            // The cost is that colour is no longer comparable between frames, which is why the
            // boundary values are surfaced in the UI (`saturation_deciles`) -- read the legend,
            // not the hue. Boundaries refresh on a slow cadence so they are legible rather than
            // strobing; between refreshes the mapping is fixed, so motion within a frame pair is
            // real motion.
            //
            // Falls back to the log-compressed absolute scale until the first refresh has run
            // (deciles empty), so the overlay is never blank.
            let w = self.heightmap.width;
            let h = self.heightmap.height;
            if self.saturation_deciles.is_empty() {
                let depth_scale = physics::REFERENCE_GRID_HEIGHT as f32 / w as f32;
                let overfill_head_unit =
                    (physics::GRAVITY_HEAD_SCALE / depth_scale) * self.overfill_stiffness;
                let base_head = physics::GRAVITY_HEAD_SCALE;
                let overfill_ratio = (self.overfill_capacity - 1.0).max(0.0);
                return (0..w * h)
                    .map(|idx| {
                        let cap = physics::cell_capacity_for(self.cell_props[idx * 4 + PROP_WETNESS]);
                        let p_val = physics::overfill_pressure_val(
                            self.heightmap.data[idx], cap, overfill_ratio, overfill_head_unit,
                            self.underfill_tension,
                        );
                        to_byte(p_val / base_head)
                    })
                    .collect();
            }
            const OCCUPIED: f32 = 1e-3;
            (0..w * h)
                .map(|idx| {
                    let h_val = self.heightmap.data[idx];
                    if h_val <= OCCUPIED || self.shape_mask[idx] == MASK_OUTSIDE {
                        return 0u8;
                    }
                    let cap = physics::cell_capacity_for(self.cell_props[idx * 4 + PROP_WETNESS]);
                    let sat = if cap > 0.0 { h_val / cap } else { 0.0 };
                    // Buckets 0..=9 spread over 1..=255; 0 is reserved for "no material", so an
                    // occupied cell in the lowest decile is still visibly distinct from air.
                    let bucket = self.saturation_bucket(sat);
                    (1 + (bucket * 254) / 9) as u8
                })
                .collect()
        } else if self.pressure_heatmap_head_field {
            crate::physics::task55_head_field::head_field_to_pressure(
                self.heightmap.width,
                self.heightmap.height,
                &self.shape_mask,
                &self.heightmap.data,
                &self.head_field,
            )
            .into_iter()
            .map(to_byte)
            .collect()
        } else {
            self.column_depth.iter().map(|&d| to_byte(d)).collect()
        }
    }

    /// Row-major coarse-level `eta` (hydraulic head) texels for the coarse-overlay debug
    /// instrument, one byte per coarse tile, always `COARSE_GRID * COARSE_GRID` -- exactly
    /// `HEAT_GRID_SIZE * HEAT_GRID_SIZE` in `sandart-render`, since the coarse grid IS the LOD
    /// block grid (`coarse::CoarseGeometry`'s `t = grid_size / 64`). This is deliberately routed
    /// through `HeightmapRenderer::update_coarse_eta`, the exact same fixed-64x64 R8Unorm upload
    /// path `block_heat_texels` above already uses (including its "no bounds check, match the
    /// size exactly" contract) -- there is no reason to invent a second shape for a texture that
    /// is already the right size.
    ///
    /// `coarse_state.eta` is `CoarseState`'s own buffer regardless of whether the coarse level is
    /// coupled into the fine solver THIS tick -- `coarse_state.tick(...)` (in `update()`, above
    /// the fine `settle_tick` call) only runs while `coarse_pressure_coupling` is on, so with that
    /// toggle off this overlay shows whatever `eta` last held (all zero if coupling has never run
    /// since the shape was last generated). That is a property of the debug toggle, not a bug
    /// here.
    ///
    /// SCALING: a FIXED physical reference, `base_head` -- "one row of gravity head" -- NOT the
    /// frame's own min/max. Per-frame min/max was tried first and rejected: it stretches
    /// whatever spread is on screen to the full ramp regardless of that spread's actual size, so
    /// a field spanning 0.0001 and one spanning 10.0 render IDENTICALLY, and worse, a nearly-flat
    /// field renders as dramatic, fully-saturated structure that is pure amplified noise. That
    /// defeats the overlay's actual job, which is to answer "does the coarse level's `eta` have a
    /// real gradient, or is it nearly flat" -- exactly the distinction per-frame normalisation
    /// erases.
    ///
    /// `base_head = GRAVITY_HEAD_SCALE * gravity_dir.y.abs()` is the SAME quantity
    /// `CoarseState::update_head_and_disagreement` (`coarse.rs`) nets out of `phi` to produce
    /// `eta` in the first place, and the same scale `physics::coarse_delta_eta` drives fine edges
    /// with -- "the scale at which `eta` differences matter" is not a free choice, it is this
    /// number. A tile-to-tile difference of one `base_head` is large: it is the head drop across
    /// one entire fine row, `coarse_delta_eta`'s standing scale for the term that drives real
    /// mass across an edge. `HALF_RANGE_BASE_HEADS` below fixes how many `base_head`s the ramp's
    /// half-width covers; this is a picked constant, not swept, and should be revisited once the
    /// user has looked at real deployed data through it.
    ///
    /// The mapping re-centres each frame on the `inside` tiles' own MEAN, not their min/max --
    /// this is not the rejected per-frame normalisation, because it only moves the ORIGIN, not
    /// the GAIN. `eta` carries an arbitrary system-wide offset (`phi`'s zero is not physically
    /// anchored), so displaying raw `eta` against an absolute zero would push an entire ordinary
    /// scene off-scale into one saturated colour; centring on the mean removes that irrelevant
    /// offset while a deviation of a given SIZE in `base_head` units always maps to the same
    /// colour shift regardless of what else is on screen -- a tiny-spread field still collapses to
    /// near-uniform grey, a genuinely sloped one still shows visible structure.
    ///
    /// Falls back to `1.0` -- the value `base_head` takes at the shipped default Sand-fall gravity
    /// (`0.04`, documented on `GRAVITY_HEAD_SCALE` as making one row's head drop exactly `1.0`),
    /// NOT an arbitrary number -- when `gravity_dir.y` is ~0 (Sandbox mode's `Phi == 0` regime),
    /// where the live "one row" scale is itself degenerate.
    ///
    /// Tiles with `open_cells[C] == 0` (`inside[C] == false` -- dry land, the exterior, or the
    /// whole grid when `coarse.available` is false at grid <= 64) are 0, the same "off/no data"
    /// convention `block_heat_texels`/`pressure_field_texels` already use for their sequential
    /// ramps.
    pub fn coarse_eta_texels(&self) -> Vec<u8> {
        const HALF_RANGE_BASE_HEADS: f32 = 1.0;
        let n = COARSE_GRID * COARSE_GRID;
        let eta = &self.coarse_state.eta;
        let inside = &self.coarse.inside;
        if eta.len() != n || inside.len() != n {
            return vec![0u8; n];
        }
        let (mean, reference) = match self.coarse_eta_stats() {
            Some((_, _, mean, reference)) => (mean, reference),
            None => return vec![0u8; n],
        };
        let half_range = HALF_RANGE_BASE_HEADS * reference;
        (0..n)
            .map(|c| {
                if !inside[c] {
                    0u8
                } else {
                    let norm = 0.5 + 0.5 * ((eta[c] - mean) / half_range).clamp(-1.0, 1.0);
                    (norm * 255.0).round() as u8
                }
            })
            .collect()
    }

    /// `eta` statistics shared by `coarse_eta_texels` (above) and the web UI's numeric readout
    /// (`sandart-wasm`'s `get_coarse_eta_stats`) -- ONE place computing min/max/mean/reference so
    /// the readout the user reads a number off of can never drift from what the colour ramp
    /// actually encoded. Returns `(min, max, mean, base_head_reference)` over `inside` tiles, or
    /// `None` when there is no `inside` tile at all (nothing coarse-coupled is on screen). See
    /// `coarse_eta_texels`'s doc comment for what `reference` means and why it is a fixed physical
    /// scale rather than the frame's own spread.
    pub fn coarse_eta_stats(&self) -> Option<(f32, f32, f32, f32)> {
        let n = COARSE_GRID * COARSE_GRID;
        let eta = &self.coarse_state.eta;
        let inside = &self.coarse.inside;
        if eta.len() != n || inside.len() != n {
            return None;
        }
        let mut min_eta = f32::INFINITY;
        let mut max_eta = f32::NEG_INFINITY;
        let mut sum = 0.0f32;
        let mut count = 0u32;
        for c in 0..n {
            if inside[c] {
                min_eta = min_eta.min(eta[c]);
                max_eta = max_eta.max(eta[c]);
                sum += eta[c];
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        let mean = sum / count as f32;
        let base_head = physics::GRAVITY_HEAD_SCALE * self.gravity_dir.y.abs();
        let reference = if base_head > 1e-3 { base_head } else { 1.0 };
        Some((min_eta, max_eta, mean, reference))
    }

    /// Row-major coarse-fine disagreement (`Delta = M - A`) texels for the coarse-overlay debug
    /// instrument. Same shape, sizing, and staleness contract as `coarse_eta_texels` just above
    /// -- read that function's doc comment first, this one only covers what differs.
    ///
    /// SCALING DIFFERS FROM `eta` in kind (diverging, not sequential) but agrees with it on the
    /// core fix: a FIXED, physically meaningful reference per tile, not a per-frame `max(|delta|)`
    /// stretch -- the same "0.01 must not look like 10" argument applies here just as much as to
    /// `eta`. The reference used is `capacity[C]`, THAT TILE's own nominal fill capacity (`M` and
    /// `A` are each individually bounded near `capacity[C]`, so `Delta = M - A` naturally ranges
    /// roughly `-capacity[C] .. +capacity[C]`): `norm = 0.5 + 0.5 * clamp(delta[C] / capacity[C],
    /// -1, 1)`. This is NOT per-frame normalisation -- `capacity[C]` is fixed geometry, unchanged
    /// tick to tick except when the shape itself is rebuilt, so a genuinely tiny disagreement
    /// reads as near-grey regardless of what any other tile or any other frame shows, and a
    /// disagreement approaching a full tile's holding capacity reads as fully saturated. 0.5
    /// (mid-ramp) is ALWAYS zero disagreement, never rescaled away from centre.
    ///
    /// Tiles with `inside[C] == false` are mapped to 128 (mid-ramp, "no disagreement"), NOT 0 like
    /// `eta`'s convention -- 0 is a real, strongly-coloured endpoint on a DIVERGING ramp (maximally
    /// negative), so using it for "no data" would paint dry land and the exterior in the same hue
    /// as the tile most starved by the coarse level, which is exactly backwards.
    pub fn coarse_delta_texels(&self) -> Vec<u8> {
        let n = COARSE_GRID * COARSE_GRID;
        let delta = &self.coarse_state.delta;
        let inside = &self.coarse.inside;
        let capacity = &self.coarse.capacity;
        if delta.len() != n || inside.len() != n || capacity.len() != n {
            return vec![0u8; n];
        }
        (0..n)
            .map(|c| {
                if !inside[c] || capacity[c] < 1e-6 {
                    128u8
                } else {
                    let norm = 0.5 + 0.5 * (delta[c] / capacity[c]).clamp(-1.0, 1.0);
                    (norm * 255.0).round() as u8
                }
            })
            .collect()
    }

    /// `max(|Delta|)` over `inside` tiles, in raw mass units -- for the web UI's numeric readout
    /// (`sandart-wasm`'s `get_coarse_delta_max_abs`) ONLY. Not used by `coarse_delta_texels`
    /// above, which normalises each tile against its OWN `capacity[C]` rather than a single
    /// scene-wide number; this is a plain absolute figure so the user can read "how big is the
    /// worst disagreement right now" directly, in the same units `Delta` itself is in. `None` when
    /// there is no `inside` tile at all.
    pub fn coarse_delta_max_abs(&self) -> Option<f32> {
        let n = COARSE_GRID * COARSE_GRID;
        let delta = &self.coarse_state.delta;
        let inside = &self.coarse.inside;
        if delta.len() != n || inside.len() != n {
            return None;
        }
        let mut max_abs = 0.0f32;
        let mut any = false;
        for c in 0..n {
            if inside[c] {
                max_abs = max_abs.max(delta[c].abs());
                any = true;
            }
        }
        if any { Some(max_abs) } else { None }
    }

    /// Recomputes `saturation_deciles` from the current heightmap. `O(n log n)` in the number of
    /// OCCUPIED cells, which is why it runs on a slow cadence (`SATURATION_DECILE_REFRESH_TICKS`)
    /// and only while the overlay is on.
    ///
    /// Empty cells are excluded deliberately. In a typical scene most of the grid is air, so
    /// including it would put every decile boundary below D8 at 0.0 and the legend would read
    /// "0, 0, 0, 0, 0, 0, 0, 0, 1.2" -- describing how much of the screen is empty, which the
    /// reader can already see, instead of how saturated the material is, which is the question.
    fn refresh_saturation_deciles(&mut self) {
        const OCCUPIED: f32 = 1e-3;
        let mut sat: Vec<f32> = self
            .heightmap
            .data
            .iter()
            .enumerate()
            .filter(|&(idx, &h)| h > OCCUPIED && self.shape_mask[idx] != MASK_OUTSIDE)
            .map(|(idx, &h)| {
                let cap = physics::cell_capacity_for(self.cell_props[idx * 4 + PROP_WETNESS]);
                if cap > 0.0 { h / cap } else { 0.0 }
            })
            .collect();

        if sat.is_empty() {
            self.saturation_deciles.clear();
            return;
        }
        sat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // TRUE NEAREST-RANK DECILES, AND NOTHING ELSE. Equalisation is the entire point of this
        // colouring -- each band must hold a tenth of the occupied cells -- and nearest rank gives
        // that by construction.
        //
        // A `MIN_BAND_SIZE` pass used to sit here, forcing consecutive boundaries at least 0.05
        // apart and, when that collapsed the count below nine, replacing the quantiles outright
        // with an evenly spaced ramp between min and max. It was added when settled water was
        // pinned against the overfill ceiling and the legend read `1.90 1.90 1.90 ...`, which it
        // did make prettier. Once the solver stopped over-compressing, EVERY pair of consecutive
        // deciles fell inside 0.05, so the redistribution path became the only path and the
        // colouring stopped being equalised at all -- `spec_task70_saturation_decile_legend`
        // caught it with 58% of occupied cells in one band.
        //
        // Near-identical numbers in the legend are now the honest reading. An incompressible fluid
        // HAS almost no spread in saturation; that is the fix working, not a display bug. If the
        // overlay needs visible structure, the quantity to colour is pressure or depth, which
        // still have a real gradient -- not a synthetic spread over one that does not.
        self.saturation_deciles = (1..10)
            .map(|d| {
                let rank = (d * sat.len()) / 10;
                sat[rank.min(sat.len() - 1)]
            })
            .collect();
    }

    /// Which decile bucket `0..=9` a saturation value falls in, given the current boundaries.
    /// `saturation_deciles` is sorted, so this is a straight scan over nine values.
    fn saturation_bucket(&self, sat: f32) -> usize {
        self.saturation_deciles.iter().take_while(|&&b| sat >= b).count()
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
        for chunk in self.cell_props.chunks_exact_mut(4) {
            chunk[PROP_WETNESS] = wetness;
            chunk[PROP_THRESHOLD] = threshold;
            chunk[PROP_FLOW_RATE] = flow_rate;
            chunk[PROP_GRAIN_SIZE] = grain_size;
        }
        self.material_mode = mode;
    }

    /// Copy color patterns into CPU color buffer
    pub fn set_cell_colors(&mut self, rgba_data: &[u8]) {
        let len = self.cell_colors.len().min(rgba_data.len());
        self.cell_colors[..len].copy_from_slice(&rgba_data[..len]);
    }

    /// Copy per-cell properties from a custom buffer
    pub fn set_cell_props(&mut self, props_data: &[f32]) {
        let len = self.cell_props.len().min(props_data.len());
        self.cell_props[..len].copy_from_slice(&props_data[..len]);
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
            | SandboxShape::MultiStageHourglass
            | SandboxShape::GaltonBoard
            | SandboxShape::StaircaseCascade
            | SandboxShape::ProceduralFunnel
            | SandboxShape::MultiNeckHourglass
            | SandboxShape::UTubeFlowThrough => {
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



        // "Perfect simulation" debug toggle: bypass the LOD scheduler's adaptive budget by
        // pre-loading every non-trivial block's recorded displacement above
        // `physics::MUST_SIMULATE_THRESHOLD` — exactly the bar `settle_tick`'s own
        // classification loop uses to admit a block into its unconditional MUST tier (see that
        // loop's doc comment in physics.rs). Routing through the SAME admission path every other
        // MUST block goes through — rather than adding a second bypass mechanism to
        // `settle_tick` itself — means `settle_tick`'s signature and its ~20 test call sites in
        // physics.rs stay untouched, and the ordinary scheduler path is provably unaffected:
        // this whole block only ever runs when `perfect_simulation` is set, and when it isn't,
        // `last_displacements` is left exactly as `settle_tick` last wrote it.
        //
        // "Non-trivial" is inside the shape mask AND holding material that could move (see
        // `PERFECT_SIM_MATERIAL_EPSILON`) — an empty block, or one entirely outside the mask, is
        // left alone; it already reads displacement 0.0 here, nowhere near the MUST bar.
        let mut perfect_sim_found_material = false;
        if self.perfect_simulation {
            // `w`/`h`/`block_size`/`cols`/`rows` are the same grid-geometry locals `update`
            // already computed above for the marble-displacement block activation, reused here
            // rather than recomputed.
            for by in 0..rows {
                for bx in 0..cols {
                    let start_x = bx * block_size;
                    let end_x = ((bx + 1) * block_size).min(w);
                    let start_y = by * block_size;
                    let end_y = ((by + 1) * block_size).min(h);
                    let mut has_material = false;
                    'scan: for y in start_y..end_y {
                        let row_offset = y * w;
                        for x in start_x..end_x {
                            let idx = row_offset + x;
                            if self.shape_mask[idx] != MASK_OUTSIDE
                                && self.heightmap.data[idx] > PERFECT_SIM_MATERIAL_EPSILON
                            {
                                has_material = true;
                                break 'scan;
                            }
                        }
                    }
                    if has_material {
                        self.last_displacements[by * cols + bx] = physics::MUST_SIMULATE_THRESHOLD;
                        perfect_sim_found_material = true;
                    }
                }
            }
        }

        // Coarse pressure state update (HIERARCHICAL-PRESSURE.md §5, build step 2 & 3).
        // Restricts fine mass into A, anchors M toward A, relaxes M across the coarse graph,
        // and updates coarse hydraulic head (eta), pressure (P), and coarse-fine disagreement (Delta).
        //
        // OVERCLOCKING.md: runs UNCONDITIONALLY now (no longer gated on `coarse_pressure_coupling`
        // -- that flag now gates only the driving-potential coupling into the fine solver, below).
        // The coarse level's own dynamics must keep running regardless: it produces `|Delta|`,
        // the multi-rate scheduler's signal, and the scheduler must work even when the potential
        // coupling is off (its shipped default, per the user's own words -- see
        // `coarse_pressure_coupling`'s doc comment).
        // LATERAL-COARSE-CORRECTION.md: arm the flow ledger for this tick, BEFORE the coarse
        // level runs, since the coarse tick is the first thing that will write to it. Sized every
        // tick rather than on resize, because `lat_ledger_enable` is also what zeroes it -- one
        // allocation-free pass over two small buffers, and it makes "the ledger holds exactly this
        // tick" true by construction rather than by remembering to clear it somewhere else.
        let correction_active = self.coarse_flow_correction
            && self.coarse.available
            && self.coarse_correction_damping > 0.0;
        // `lat_ledger_ensure`, not `lat_ledger_enable`: the fine half must SURVIVE into this tick,
        // because the boost below compares this tick's coarse transport against last tick's fine
        // transport. Each half is zeroed explicitly just before the thing that writes it runs.
        physics::lat_ledger_ensure(
            correction_active,
            self.coarse_state.delta.len(),
            self.active_blocks.len(),
        );
        if correction_active {
            // The coarse tick is about to rewrite the coarse half.
            physics::lat_ledger_clear_coarse();
        }
        if !correction_active {
            self.last_frame_correction = physics::LateralCorrectionStats::default();
        }

        if self.coarse.available {
            // STEP4-COARSE-IS-A-SIM.md: the coarse level's own dynamics now run the shipped
            // solver over a nested grid (`CoarseState::advance_nested_sim`), so it needs the
            // real `gravity_dir` (not a pre-scaled scalar `base_head`) and the real overfill
            // settings, unmodified -- "the coarse sim IS a 64x64 sim" means it uses the same
            // constants the fine level does, derived the same way. `unit` remains the FINE
            // grid's own `overfill_head_unit`, used only for `eta`'s export-side pressure scaling
            // (unchanged from before this rebuild).
            let expanded = Self::expand_touched_to_tiles(
                &self.blocks_touched,
                self.heightmap.width,
                self.block_size,
            );
            let depth_scale = physics::REFERENCE_GRID_HEIGHT as f32 / self.heightmap.width as f32;
            let unit = (physics::GRAVITY_HEAD_SCALE / depth_scale) * self.overfill_stiffness;
            let overfill_ratio = (self.overfill_capacity - 1.0).max(0.0);
            self.coarse_state.tick(
                &self.heightmap.data,
                &self.shape_mask,
                &self.cell_props,
                &self.coarse,
                self.gravity_dir,
                overfill_ratio,
                self.underfill_tension,
                self.overfill_stiffness,
                unit,
                // `restrict_incremental` indexes this by COARSE TILE, and at the default block
                // divisor a block and a tile are the same square, so this is the identity and
                // `expanded` stays `None` -- no clone on the shipped path. At any other divisor
                // the mask is expanded rather than handed over at the wrong length, which
                // `restrict_incremental` would (correctly but expensively) treat as "cannot
                // vouch for this" and answer with a full restrict every tick.
                Some(expanded.as_deref().unwrap_or(&self.blocks_touched)),
            );
            // Everything the coarse level moved this tick is now in the ledger's COARSE half;
            // everything the fine level moves below belongs in the FINE half.
            physics::lat_ledger_set_coarse(false);
        }

        // LATERAL-COARSE-CORRECTION.md: turn this tick's coarse-vs-fine lateral deficit into a
        // per-block conveyance multiplier, and install it for the repetitions below.
        //
        // Ordering: the coarse level has just run, so `coarse_h` is THIS tick's transport, while
        // `fine_h` is still last tick's -- the fine level has not run yet. That is the right
        // pairing rather than a lag to apologise for: the boost has to be in place BEFORE
        // `settle_tick` so the fine solver can act on it, and last tick's realised lateral flow is
        // the best available statement of what the fine level manages at this configuration.
        //
        // The boost changes only how fast the fine solver may convey laterally. It moves no mass
        // and decides no placement -- see `compute_lateral_boost` for why that distinction is the
        // whole design.
        if correction_active {
            let (coarse_h, coarse_v, fine_h, fine_v) = physics::lat_ledger_snapshot();
            let (boost_h, boost_v, stats) = physics::compute_lateral_boost(
                self.heightmap.width,
                self.heightmap.height,
                self.block_size,
                self.coarse.coarse_n,
                &coarse_h,
                &coarse_v,
                &fine_h,
                &fine_v,
                self.coarse_correction_damping,
            );
            self.last_frame_correction = stats;
            physics::set_lateral_boost(
                &boost_h,
                if self.coarse_correction_vertical { &boost_v } else { &[] },
            );
            // Last tick's fine transport has now been consumed; the repetitions below accumulate
            // this tick's into a clean buffer.
            physics::lat_ledger_clear_fine();
        } else {
            physics::set_lateral_boost(&[], &[]);
        }

        // OVERCLOCKING.md (HIERARCHICAL-PRESSURE.md §7b): update the multi-rate block scheduler.
        // Defensive resize first -- mirrors `settle_tick`'s own belt-and-suspenders pattern for
        // `last_displacements`/`active_blocks`, in case the block grid ever changed shape without
        // going through `new_with_size`/`reset()` (neither of which happens in production today).
        let expected_block_len = self.active_blocks.len();
        if self.block_clock_rate.len() != expected_block_len {
            self.block_clock_rate.resize(expected_block_len, 1.0);
        }
        if self.overclocking_enabled && self.coarse.available {
            self.update_block_clock_rates();
            self.apply_underclock_skip();
        } else if self.block_clock_rate.iter().any(|&r| r != 1.0) {
            // Bit-identical to the toggle never having existed: no stale rate lingers on screen
            // or in the scheduler while this is off.
            self.block_clock_rate.fill(1.0);
        }
        // Upper bound on EXTRA `settle_tick` repetitions this frame, beyond the normal one -- a
        // block at rate `n` (n > 1) runs AT MOST `round(n)` real sub-steps total this frame
        // (EARLY-STOP.md: `force_overclocked_blocks_active` stops re-forcing a block once its own
        // real displacement falls under `MUST_SIMULATE_THRESHOLD`, so most blocks run fewer than
        // their rate's worth of repetitions -- see that function's doc comment for why arbitrary,
        // non-power-of-two rates are still safe here). This frame-wide `extra_reps` is still the
        // MAXIMUM rate across all blocks, rounded -- the rep loop below still iterates that many
        // times, it just skips a settled block's own interior work on each one it no longer needs.
        let extra_reps: u32 = if self.overclocking_enabled {
            self.block_clock_rate
                .iter()
                .cloned()
                .fold(1.0f32, f32::max)
                .round()
                .max(1.0) as u32
                - 1
        } else {
            0
        };

        // Run the gravity-driven settling cellular automata tick
        //
        // `perfect_sim_found_material` is OR'd in explicitly rather than relying on the injected
        // displacement value alone to trip the `> 3e-4` check just below: `settle_tick`'s own
        // MUST bar (`physics::MUST_SIMULATE_THRESHOLD` = 1e-4) sits below this gate's 3e-4 by
        // design (see that constant's doc comment), so a freshly-injected 1e-4 would silently
        // fail to mark the tick active without this. `extra_reps > 0` is OR'd in for the same
        // reason: overclocking can want extra sub-steps even on a frame where nothing has yet
        // crossed the ordinary activity bars.
        let has_active = perfect_sim_found_material
            || self.last_displacements.iter().any(|&x| x > 3e-4)
            || self.marbles.iter().any(|m| m.was_active)
            || self.gravity_dir.length_squared() > 1e-6
            || extra_reps > 0;
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

            // OVERCLOCKING.md: `rep in 0..=extra_reps` runs the normal call (`rep == 0`,
            // unchanged classification, exactly as before this feature existed when
            // `extra_reps == 0`) plus one real `settle_tick` call per additional sub-step a
            // genuinely overclocked block has earned. `self.tick_count` is passed UNCHANGED on
            // every repetition (not `+ rep`) -- F1 (HIERARCHICAL-PRESSURE.md §7b): staleness
            // (`MAX_STALENESS` in physics.rs) is counted in `tick_count`, and re-using the same
            // value for every repetition within one rendered frame means a tick keeps meaning
            // "one rendered frame" regardless of how many internal sub-steps ran this frame, so
            // `MAX_STALENESS` means the same amount of simulated time at every clock setting.
            // `time_seed` still varies per repetition so repeated sub-steps do not all draw the
            // exact same jitter/parity.
            //
            // `touched_this_rep` (rather than passing `&mut self.blocks_touched` directly) and
            // the OR-merge below are necessary once `extra_reps > 0`: `settle_tick`'s
            // `touched_out` REPLACES its target's contents each call, so passing
            // `self.blocks_touched` straight through would let the LAST repetition's touched set
            // silently overwrite (not accumulate with) every earlier repetition's -- and
            // `CoarseState::restrict_incremental` (next tick) trusts that set completely, so a
            // block touched only in an earlier repetition would read as unchanged and never get
            // re-aggregated into `A[C]`. Reduces to one call and one copy, bit-identical to
            // before this feature existed, when `extra_reps == 0`.
            let mut touched_accum: Vec<bool> = Vec::new();
            // PERF-PROFILE.md TEMPORARY: first rep (per block) whose real `last_displacements`
            // fell under threshold, -1 if never within this frame's rep budget.
            let mut first_settled: Vec<i32> = vec![-1; self.block_clock_rate.len()];
            // EARLY-STOP.md: the ACTUAL count of per-block interior sweeps this frame, summed
            // across every repetition -- see `last_frame_block_steps`'s own doc comment.
            let mut block_steps_this_frame: u32 = 0;
            // EARLY-STOP.md: see `last_frame_stalled_boundaries`.
            let mut stalled_boundaries: u32 = 0;
            // EARLY-STOP.md: see `last_frame_block_substeps`. Zeroed here, not in the per-rep
            // loop, so it accumulates across every repetition of THIS frame and nothing else.
            if self.last_frame_block_substeps.len() != self.active_blocks.len() {
                self.last_frame_block_substeps.resize(self.active_blocks.len(), 0);
            }
            self.last_frame_block_substeps.iter_mut().for_each(|v| *v = 0);
            // CLASSIFICATION-HOIST.md Stage 1: `fresh_overburden_must_blocks`/`support_fraction`
            // classification was measured at ~54% of an overclocked frame (SCAFFOLDING-BREAKDOWN.md),
            // paid identically on every one of this frame's `extra_reps + 1` `settle_tick` calls for
            // an answer that barely changes rep-over-rep (the `needed[]` mask does not shrink
            // materially -- same doc, and physics.rs's `compute_fresh_active` doc comment). Computed
            // ONCE here, on the state as it stands before this frame's first `settle_tick` call
            // (identical to what rep 0 would have computed live), and reused for every repetition.
            // Safe by construction: the predicate "only ever adds indices to `must_simulate`; it
            // never feeds a physics quantity" (physics.rs, `fresh_overburden_must_blocks`'s own doc
            // comment) -- caching it changes WHICH blocks get scheduled, never what the physics
            // computes. Verified against the defect this predicate exists to fix (Task #47 sand-slab
            // divergence vs. perfect-sim) rather than assumed -- see CLASSIFICATION-HOIST.md.
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
            for rep in 0..=extra_reps {
                // EARLY-STOP.md: blocks sitting out this repetition under `rate_gated_reps`, as
                // `(block, displacement before it was zeroed)`. Restored after the call.
                let mut stashed: Vec<(usize, f32)> = Vec::new();
                if rep > 0 {
                    let participating = self.force_overclocked_blocks_active(rep);
                    if self.rate_gated_reps {
                        // Count the seams this repetition's suppression creates BEFORE acting on
                        // it: an edge whose owning block (left/top, since edges belong to their
                        // lower-index cell) sits out while the far side runs cannot be evaluated
                        // by anyone this repetition.
                        let cols_b = self.block_size_cols();
                        if cols_b > 0 {
                            let rows_b = participating.len() / cols_b;
                            for b in 0..participating.len() {
                                let bx = b % cols_b;
                                let by = b / cols_b;
                                if participating[b] {
                                    continue;
                                }
                                if bx + 1 < cols_b && participating[b + 1] {
                                    stalled_boundaries += 1;
                                }
                                if by + 1 < rows_b && participating[b + cols_b] {
                                    stalled_boundaries += 1;
                                }
                            }
                        }
                        // Zeroing the displacement is the same lever `apply_underclock_skip`
                        // uses: it is what keeps a block out of `settle_tick`'s MUST and budget
                        // tiers. STALE is deliberately still reachable -- `MAX_STALENESS` is the
                        // independent backstop and gating it on a clock rate would remove the one
                        // mechanism that catches a wrongly-suppressed block.
                        for b in 0..participating.len().min(self.last_displacements.len()) {
                            if !participating[b] && self.last_displacements[b] != 0.0 {
                                stashed.push((b, self.last_displacements[b]));
                                self.last_displacements[b] = 0.0;
                            }
                        }
                    }
                }
                let mut touched_this_rep: Vec<bool> = Vec::new();
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
                    time_seed.wrapping_add(rep.wrapping_mul(0x9E37_79B1)),
                    &mut self.edge_vel_h,
                    &mut self.edge_vel_v,
                    &mut self.column_depth,
                    &mut self.head_field,
                    &self.shape_mask,
                    self.tick_count,
                    self.gravity_dir,
                    self.fresh_pressure_field,
                    self.head_field_transport,
                    self.pressure_heatmap_head_field,
                    self.pressure_sensitive_flow,
                    self.overfill_pressure,
                    (self.overfill_capacity - 1.0).max(0.0),
                    self.underfill_tension,
                    self.overfill_stiffness,
                    // Empty (not `&self.coarse_state.eta`) whenever the coarse level itself says
                    // it is not coupled this tick -- `coarse_delta_eta` in physics.rs relies on
                    // emptiness, not buffer length, to detect "not available", since
                    // `CoarseState`'s buffers stay sized `COARSE_GRID * COARSE_GRID` regardless.
                    // Also empty whenever the debug toggle is off (see `coarse_pressure_coupling`'s
                    // own doc comment) -- same signal, same reason: `settle_tick` must take the
                    // exact pre-coupling code path when the toggle is off.
                    if self.coarse.available && self.coarse_pressure_coupling { &self.coarse_state.eta } else { &[] },
                    // Same emptiness contract, for I4's per-tile flux budget (§6): `|Delta[C]|`
                    // per tile, consumed by `coarse_delta_eta_budgeted` in physics.rs.
                    if self.coarse.available && self.coarse_pressure_coupling { &self.coarse_state.delta } else { &[] },
                    // CLASSIFICATION-HOIST.md Stage 1: the same frame-wide mask on every repetition
                    // (see its own binding just above the `for rep` loop for why this is safe).
                    Some(&fresh_active),
                    Some(&mut touched_this_rep),
                    self.liquid_fall_jitter,
                );
                // EARLY-STOP.md: `active_blocks[b] != Inactive` is exactly `will_simulate[b]` from
                // this call (see `settle_tick`'s classification loop in physics.rs, which writes
                // both from the same `must_simulate`/`stale_simulate`/`budget_simulate` sets) --
                // i.e. this counts real interior sweeps actually run this repetition, not the
                // repetition's rate-implied budget.
                block_steps_this_frame += self
                    .active_blocks
                    .iter()
                    .filter(|&&a| a != BlockActivity::Inactive)
                    .count() as u32;
                // Same predicate, per block rather than summed -- see `last_frame_block_substeps`.
                for (b, &a) in self.active_blocks.iter().enumerate() {
                    if a != BlockActivity::Inactive {
                        self.last_frame_block_substeps[b] += 1;
                    }
                }
                // Restore what was stashed above. `max()`, not assignment: a suppressed block can
                // still RECEIVE mass from a running neighbour (S2), and that transfer writes a
                // live displacement this must not overwrite with the stale snapshot. Taking the
                // larger of the two can only ever schedule a block MORE eagerly, which is the
                // safe direction -- the failure this guards against is a block that moved being
                // recorded as settled.
                for &(b, prev) in &stashed {
                    self.last_displacements[b] = self.last_displacements[b].max(prev);
                }
                // PERF-PROFILE.md TEMPORARY: capture the REAL post-settle_tick displacement for
                // this rep, before the next iteration's `force_overclocked_blocks_active` (if
                // any) overwrites it.
                for b in 0..self.block_clock_rate.len().min(self.last_displacements.len()) {
                    if self.block_clock_rate[b] > 1.0
                        && first_settled[b] < 0
                        && self.last_displacements[b] < physics::MUST_SIMULATE_THRESHOLD
                    {
                        first_settled[b] = rep as i32;
                    }
                }
                if touched_accum.len() != touched_this_rep.len() {
                    touched_accum.resize(touched_this_rep.len(), false);
                }
                for (acc, &t) in touched_accum.iter_mut().zip(touched_this_rep.iter()) {
                    *acc |= t;
                }
            }
            self.blocks_touched = touched_accum;
            self.last_frame_block_steps = block_steps_this_frame;
            self.last_frame_stalled_boundaries = stalled_boundaries;
            // PERF-PROFILE.md TEMPORARY: log (target sub-steps, first-settled rep or -1) for
            // every block that was genuinely overclocked this frame.
            if extra_reps > 0 {
                EARLY_TERM_LOG.with(|c| {
                    let mut log = c.borrow_mut();
                    for b in 0..self.block_clock_rate.len() {
                        let rate = self.block_clock_rate[b];
                        if rate > 1.0 {
                            log.push((rate.round() as u32, first_settled[b]));
                        }
                    }
                });
            }
        } else {
            self.active_bounds.active = false;
            // Nothing ran, so no block's fine heights could have changed -- see
            // `blocks_touched`'s own doc comment for why stale content here (rather than
            // all-false) would be merely wasteful, not incorrect, but is cleared anyway.
            self.blocks_touched.iter_mut().for_each(|v| *v = false);
            self.last_frame_block_steps = 0;
            self.last_frame_stalled_boundaries = 0;
            self.last_frame_block_substeps.iter_mut().for_each(|v| *v = 0);
        }

        self.tick_count = self.tick_count.wrapping_add(1);

        // Block-simulation heat-map bookkeeping (debug overlay) — see `block_heat_buckets`'s
        // doc comment for exactly what this counts and the bucket-decay approximation it makes.
        // Runs every tick regardless of `has_active` so the trailing window keeps aging stale
        // activity out even while the sim is fully at rest; only the increment step below is
        // gated on `has_active`, since `active_blocks` was NOT refreshed by `settle_tick` this
        // tick if it didn't run, and crediting its stale contents into the new bucket would
        // double-count activity that actually happened one or more ticks ago.
        if self.block_heat_buckets.len() != self.active_blocks.len() * HEAT_NUM_BUCKETS {
            self.block_heat_buckets
                .resize(self.active_blocks.len() * HEAT_NUM_BUCKETS, 0);
        }
        let heat_bucket = ((self.tick_count / HEAT_BUCKET_TICKS) % HEAT_NUM_BUCKETS as u32) as usize;
        if self.tick_count % HEAT_BUCKET_TICKS == 0 {
            // Entering a new bucket: this slot last held the chunk that is now 300 ticks old,
            // so clear it before this tick's activity starts filling it back in.
            for b in 0..self.active_blocks.len() {
                self.block_heat_buckets[b * HEAT_NUM_BUCKETS + heat_bucket] = 0;
            }
        }
        if has_active {
            for (b, &activity) in self.active_blocks.iter().enumerate() {
                if activity != BlockActivity::Inactive {
                    let slot = &mut self.block_heat_buckets[b * HEAT_NUM_BUCKETS + heat_bucket];
                    *slot = slot.saturating_add(1);
                }
            }
        }

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

        // Saturation deciles for the overfill heat-map. Gated on the overlay actually being on,
        // so a normal run never pays for it, and refreshed on a slow cadence because these are a
        // legend the reader is reading, not a per-frame signal -- a scale that jumps every frame
        // is unreadable and makes two frames incomparable.
        if self.overfill_pressure && self.pressure_heatmap_overlay
            && self.tick_count % SATURATION_DECILE_REFRESH_TICKS == 0
        {
            self.refresh_saturation_deciles();
        }

        // Update EMA of frame time and adjust budget_n
        const EMA_ALPHA: f32 = 0.1;
        // 4x their pre-#(this task) values (32 / 4 / 1) — block counts quadrupled (1024 -> 4096
        // at grid >= 128, see `block_size`'s doc comment), and these are block-count throttles, so
        // leaving them unscaled would have made the adaptive controller 4x tighter (floor) and 4x
        // slower to respond (step) as a *fraction* of the block grid than it was before this
        // change, with no corresponding change in actual physics cost per block.
        const BUDGET_MIN: usize = 128;
        const BUDGET_STEP_DOWN: usize = 16;
        const BUDGET_STEP_UP: usize = 4;

        if last_frame_time_ms > 0.0 && target_frame_time_ms > 0.0 {
            self.ema_frame_ms = EMA_ALPHA * last_frame_time_ms + (1.0 - EMA_ALPHA) * self.ema_frame_ms;
            
            let budget_max = cols * rows; // e.g. 1024

            // Target 95% of target FPS (Vsync interval * 1.05) to account for browser Vsync-locking
            // and allow the budget to grow back up when running smoothly.
            let adjusted_target = target_frame_time_ms * 1.05;

            if self.ema_frame_ms > adjusted_target {
                self.budget_n = self.budget_n.saturating_sub(BUDGET_STEP_DOWN).max(BUDGET_MIN);
            } else if self.ema_frame_ms < adjusted_target {
                self.budget_n = (self.budget_n + BUDGET_STEP_UP).min(budget_max);
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
        SandboxShape::MultiStageHourglass,
        SandboxShape::GaltonBoard,
        SandboxShape::StaircaseCascade,
        SandboxShape::ProceduralFunnel,
        SandboxShape::MultiNeckHourglass,
        SandboxShape::UTubeFlowThrough,
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
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
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
                let r = s.cell_colors[idx * 4 + 0] as f64;
                let g = s.cell_colors[idx * 4 + 1] as f64;
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
    // `test_cascade_no_sand_leaking` pinned exactly one shape. Every funnel geometry has the
    // same failure mode — a shelf, peg or neck that does not quite close lets sand cross into
    // MASK_OUTSIDE, where `settle_tick`'s mask guards freeze it permanently — so all of them are
    // worth the same check, and the ones whose geometry just changed most of all: the Galton peg
    // lattice, the three-neck hourglass and the finer staircase.
    //
    // Run at the default neck width only — sweeping the slider here costs 20s of suite time, and
    // what the slider actually threatens is *geometric* (necks merging, shelves fusing into a
    // slab). That is covered per-shape instead, for free, by mask inspection:
    // `test_staircase_steps_stay_separated` and
    // `test_cascade_no_dam_or_neck_merge_across_full_slider_range`.
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
    // geometry does not depend on the neck slider. The cascade's slider sweep lives in
    // `test_cascade_no_dam_or_neck_merge_across_full_slider_range`.
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
            SandboxShape::MultiStageHourglass,
            SandboxShape::ProceduralFunnel,
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
    fn test_cascade_no_sand_leaking() {
        let mut sim = super::DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::MultiStageHourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();

        let initial_mass: f32 = sim.heightmap.data.iter().sum();
        assert!(initial_mass > 0.0, "MultiStageHourglass should be initialized with sand in tier 0");

        let targets = [None; 5];
        // Run gravity simulation for 500 ticks across all 4 tiers
        for _ in 0..500 {
            sim.update(
                0.016,
                &targets,
                0.08,
                MaterialMode::DrySand,
                SandboxShape::MultiStageHourglass,
                16.0,
                16.0,
            );
        }

        let final_mass: f32 = sim.heightmap.data.iter().sum();
        let mass_err = (final_mass - initial_mass).abs() / initial_mass;

        // Verify 100.0000% sand mass conservation with ZERO leaks out of bounds
        assert!(
            mass_err < 0.0001,
            "Cascade sandbox leaked sand mass under gravity! Init={:.4}, Final={:.4}, Error={:.6}",
            initial_mass,
            final_mass,
            mass_err
        );
    }

    // The "does sand actually reach the bottom chamber" check lives in physics.rs as
    // `test_cascade_drains_to_bottom_chamber`, alongside `test_hourglass_full_drainage` which it
    // mirrors -- both drive `settle_tick` directly on a small custom grid instead of the full
    // `DrawingSimulation` pipeline, which is the difference between this suite taking seconds and
    // taking minutes.

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
    fn test_galton_board_has_no_clear_vertical_shafts() {
        let mut sim = super::DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::GaltonBoard;
        sim.generate_shape_mask();

        let w = GRID_SIZE;
        let h = GRID_SIZE;
        // The peg field lives below the neck, in `dy` in (6, 0.38 * h) — see the GaltonBoard arm
        // of `eval_sandbox_shape`. Sample the interior of that band only, so the funnel's own
        // taper cannot be mistaken for an obstruction.
        let y_lo = h / 2 + 8;
        let y_hi = h / 2 + (0.34 * h as f32) as usize;

        let mut open_shafts = Vec::new();
        for x in 0..w {
            // Only columns that are actually open at the top of the band can be a shaft; a column
            // buried in the wall is not sand's path.
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
            "Sand falls straight through the Galton board at {} column(s) {:?} — no peg obstructs \
             them anywhere between rows {} and {}",
            open_shafts.len(),
            open_shafts,
            y_lo,
            y_hi
        );
    }

    #[test]
    #[ignore = "DIAGNOSTIC (Stage C task report): does DrySand still scatter across the Galton \
                board's pegs -- the shape whose whole visual point is a spread bottom \
                distribution -- once its lateral transport runs on the yield-stress flux edge \
                instead of the old CA's stochastic dispersion? Not a behavioural spec (no fixed \
                pass/fail bar on the spread number itself, since there was never a prior \
                assertion establishing what it should be), just a printed measurement to compare \
                against a pre-Stage-C checkout. Run with --ignored --nocapture."]
    fn diag_galton_board_bottom_spread() {
        let mut sim = DrawingSimulation::new();
        sim.sandbox_shape = SandboxShape::GaltonBoard;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(MaterialMode::DrySand);
        sim.initialize_hourglass();

        let targets = [None; 5];
        for _ in 0..1500 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::GaltonBoard, 0.0, 16.0);
        }

        let w = sim.heightmap.width;
        let h = sim.heightmap.height;
        // Bottom collection zone: below the peg field (see the shafts test above for where the
        // peg field itself lives, `dy` in (6, 0.38 * h) below the neck).
        let y_lo = h / 2 + (0.40 * h as f32) as usize;
        let mut col_mass = vec![0.0f64; w];
        let mut total = 0.0f64;
        let mut occupied_cols = 0usize;
        for x in 0..w {
            let mut m = 0.0f64;
            for y in y_lo..h {
                m += sim.heightmap.data[y * w + x] as f64;
            }
            col_mass[x] = m;
            total += m;
            if m > 0.01 {
                occupied_cols += 1;
            }
        }
        let mean_x: f64 = if total > 0.0 {
            col_mass.iter().enumerate().map(|(x, &m)| x as f64 * m).sum::<f64>() / total
        } else {
            f64::NAN
        };
        let var_x: f64 = if total > 0.0 {
            col_mass.iter().enumerate().map(|(x, &m)| (x as f64 - mean_x).powi(2) * m).sum::<f64>() / total
        } else {
            f64::NAN
        };
        // Peak concentration: the single fullest column's share of the bottom-zone mass. A
        // Galton board that scatters should have this well under 1.0 (mass spread over many
        // columns); a board that funnels straight down a single channel would have it near 1.0.
        let peak_frac = col_mass.iter().cloned().fold(0.0f64, f64::max) / total.max(1e-12);
        println!(
            "diag_galton_board_bottom_spread: total_mass={:.2} occupied_cols={} mean_x={:.2} \
             std_x={:.2} (grid w={}) peak_col_frac={:.4}",
            total, occupied_cols, mean_x, var_x.sqrt(), w, peak_frac
        );
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
}


