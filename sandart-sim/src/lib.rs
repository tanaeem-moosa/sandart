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
    /// rather than staying an absolute constant, specifically `(grid_size / 32).max(1)`, so the
    /// grid is always tiled into the same 32x32 = 1024 blocks regardless of resolution. This
    /// keeps `budget_n` (and `BUDGET_MIN`/`BUDGET_STEP_*` in `update`) meaningful as the *same
    /// fraction* of the grid at every resolution. Keeping `block_size` absolute instead would
    /// have made low resolutions (e.g. 64/16 = 4x4 = 16 total blocks) fall entirely under
    /// `budget_n`'s minimum, disabling the LOD scheduler's throttling outright at low res and
    /// making 64 behave differently from 512 for scheduling reasons unrelated to physics.
    pub fn new_with_size(grid_size: usize) -> Self {
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
        // therefore the meaning of budget_n) stays resolution-invariant.
        let block_size = (grid_size / 32).max(1);
        let cols = (grid_size + block_size - 1) / block_size;
        let rows = (grid_size + block_size - 1) / block_size;
        let active_blocks = vec![BlockActivity::Inactive; cols * rows];
        let last_displacements = vec![0.0f32; cols * rows];
        let last_simulated_ticks = vec![0u32; cols * rows];
        let budget_n = 256;
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
        self.budget_n = 256;
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

    /// Row-major block-heat texel bytes for the heat-map debug overlay, one byte per block
    /// (always a 32x32 grid — see `new_with_size`'s doc comment), ready for direct upload as an
    /// R8Unorm GPU texture: `byte = round((times_simulated_in_window / 300) * 255)`, clamped.
    /// See `block_heat_buckets` for exactly what "times simulated in window" means and the
    /// approximation it makes versus a true 300-tick trailing count.
    pub fn block_heat_texels(&self) -> Vec<u8> {
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

        // Run the gravity-driven settling cellular automata tick
        //
        // `perfect_sim_found_material` is OR'd in explicitly rather than relying on the injected
        // displacement value alone to trip the `> 3e-4` check just below: `settle_tick`'s own
        // MUST bar (`physics::MUST_SIMULATE_THRESHOLD` = 1e-4) sits below this gate's 3e-4 by
        // design (see that constant's doc comment), so a freshly-injected 1e-4 would silently
        // fail to mark the tick active without this.
        let has_active = perfect_sim_found_material
            || self.last_displacements.iter().any(|&x| x > 3e-4)
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

            let iterations = if self.gravity_dir.length_squared() > 1e-6 { 1 } else { 1 }; // STAGE3 PROBE
            for iter in 0..iterations {
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
                    time_seed + iter as u32,
                    &mut self.edge_vel_h,
                    &mut self.edge_vel_v,
                    &mut self.column_depth,
                    &mut self.head_field,
                    &self.shape_mask,
                    self.tick_count + iter as u32,
                    self.gravity_dir,
                    self.fresh_pressure_field,
                    self.head_field_transport,
                    self.pressure_heatmap_head_field,
                    self.pressure_sensitive_flow,
                    self.overfill_pressure,
                    (self.overfill_capacity - 1.0).max(0.0),
                    self.underfill_tension,
                    self.overfill_stiffness,
                );
            }
        } else {
            self.active_bounds.active = false;
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
        const BUDGET_MIN: usize = 32;
        const BUDGET_STEP_DOWN: usize = 4;
        const BUDGET_STEP_UP: usize = 1;

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


