//! Constants copied verbatim from `sandart-sim/src/physics.rs`. Kept in one place so every
//! kernel (R, A, B) reads the identical numeric constant a real lateral pass would.
//!
//! This crate does NOT depend on `sandart-sim` (see `lib.rs`'s module doc comment for why), so
//! there is no way to `pub use` these from the original crate; they are transcribed by hand.
//! If `physics.rs` ever changes one of these, this file goes stale silently -- that is an
//! accepted risk for a benchmarking harness that must not touch production code, not an
//! oversight.

/// `physics::PROP_WETNESS`.
pub const PROP_WETNESS: usize = 0;
/// `physics::PROP_THRESHOLD`.
pub const PROP_THRESHOLD: usize = 1;
/// `physics::PROP_FLOW_RATE` (unused by the lateral edge, kept for layout completeness).
#[allow(dead_code)]
pub const PROP_FLOW_RATE: usize = 2;
/// `physics::PROP_GRAIN_SIZE`.
pub const PROP_GRAIN_SIZE: usize = 3;

/// `physics::MASK_OUTSIDE`.
pub const MASK_OUTSIDE: u8 = 0;

/// `physics::LATERAL_PRESSURE_SCALE`.
pub const LATERAL_PRESSURE_SCALE: f32 = 5.0;
/// `physics::GRANULAR_TAU_SCALE`.
pub const GRANULAR_TAU_SCALE: f32 = 1.0;
/// `physics::LATERAL_EARTH_PRESSURE_K`.
pub const LATERAL_EARTH_PRESSURE_K: f32 = 0.45;
/// `physics::JANSSEN_DEPTH_SCALE`.
pub const JANSSEN_DEPTH_SCALE: f32 = 24.0;
/// `physics::GRAVITY_HEAD_SCALE`.
pub const GRAVITY_HEAD_SCALE: f32 = 25.0;
/// `physics::DISPERSION_TAU_FRAC`.
pub const DISPERSION_TAU_FRAC: f32 = 0.5;
/// `physics::GRAVITY_LOCK_CHANCE`.
pub const GRAVITY_LOCK_CHANCE: f32 = 0.05;
/// `physics::REFERENCE_GRID_HEIGHT`.
pub const REFERENCE_GRID_HEIGHT: usize = 512;
/// `physics::GRAIN_JITTER_SCALE`.
pub const GRAIN_JITTER_SCALE: f32 = 1.25;
/// `physics::GRAIN_JITTER_MAX`.
pub const GRAIN_JITTER_MAX: f32 = 0.95;
/// `physics::EDGE_SALT_H`.
pub const EDGE_SALT_H: u32 = 0x27d4_eb2f;
/// `flux_edge_apply`'s `MIN_FLUX`.
pub const MIN_FLUX: f32 = 1e-7;

/// The phase this benchmark always ports: the ONE base lateral pass (real `settle_tick`'s
/// `phase == 1`), never an extra `lateral_substeps` pass (`phase >= 2`). `weight` is therefore
/// always `1.0` and `cand_h_unweighted` never comes up -- see `lib.rs`'s module doc comment.
pub const PHASE: u32 = 1;
