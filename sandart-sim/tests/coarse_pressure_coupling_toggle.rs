//! Empirical proof of three things about the "coarse pressure coupling" debug toggle
//! (`DrawingSimulation::coarse_pressure_coupling`, HIERARCHICAL-PRESSURE.md, OVERCLOCKING.md):
//!
//! 1. Left at its default (`false`, since OVERCLOCKING.md split this toggle's meaning -- see the
//!    field's own doc comment) is indistinguishable from explicitly setting it `false`.
//! 2. Explicitly setting it `true` actually changes behaviour (the toggle is not a placebo).
//! 3. Setting it `false` is BIT-IDENTICAL to the coarse level being structurally unavailable
//!    (`CoarseGeometry::available == false`, the same "not coupled" signal grid 64 has always
//!    produced, predating this toggle) -- the strongest evidence available from this crate's
//!    public surface that the OFF path reproduces pre-coupling behaviour exactly on the buffers
//!    this checksum covers. This still holds even though the coarse level's OWN per-tick
//!    dynamics (`coarse_state.tick`) now run unconditionally whenever `coarse.available` (no
//!    longer gated by this toggle -- OVERCLOCKING.md, so the multi-rate scheduler can see
//!    `|Delta|` regardless of whether the driving-potential coupling is on): that machinery only
//!    ever writes into `coarse_state`'s own buffers, which this checksum does not cover, and it
//!    reaches `heightmap`/`cell_colors`/`cell_props` (what IS checksummed) only via
//!    `coarse_eta`/`coarse_delta` passed to `settle_tick`, which stay empty whenever
//!    `coarse_pressure_coupling` is off.
//!
//! This mirrors `fresh_pressure_field_toggle.rs`/`overfill_pressure_toggle.rs`'s
//! `*_left_untouched_matches_explicitly_disabled` / `*_enabled_diverges_from_default` pattern --
//! now the same shape as its siblings, since this toggle no longer defaults ON.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// FNV-1a checksum over every buffer `update` can mutate -- same construction as the sibling
/// toggle tests, sensitive enough that a single flipped bit anywhere changes it.
fn checksum(sim: &DrawingSimulation) -> u64 {
    fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    hash = fnv1a(bytemuck_cast_f32(&sim.heightmap.data), hash);
    hash = fnv1a(&sim.cell_colors, hash);
    hash = fnv1a(bytemuck_cast_f32(&sim.cell_props), hash);
    hash
}

fn bytemuck_cast_f32(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}

/// Grid 128 (t=2, `coarse.available == true`) so the coarse level is actually engaged, U-tube so
/// there is genuine, sustained coarse-fine disagreement (a draining reservoir feeding a rising
/// riser through a saturated basin) for the coupling to have visible leverage over -- the same
/// scenario `spec_task70_u_tube_riser_keeps_rising` uses. `overfill_pressure` must be on: the
/// coarse term only enters through the overfill branch of the edge solver (see
/// `physics::settle_tick`'s `if overfill_active { ... coarse_delta_eta_budgeted ... }`).
fn run(touch: impl FnOnce(&mut DrawingSimulation), ticks: usize) -> u64 {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.overfill_capacity = 1.90;
    sim.initialize_hourglass();
    touch(&mut sim);

    let targets = [None; 5];
    for _ in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
    }
    checksum(&sim)
}

#[test]
fn coarse_pressure_coupling_left_untouched_matches_explicitly_disabled() {
    let untouched = run(|_sim| {}, 500);
    let explicitly_off = run(|sim| sim.coarse_pressure_coupling = false, 500);
    assert_eq!(
        untouched, explicitly_off,
        "never touching coarse_pressure_coupling must be indistinguishable from explicitly \
         setting it false -- the field's default (set in DrawingSimulation::new_with_size) must \
         be false (OVERCLOCKING.md: 'let's leave the coupling behind a flag until we are happy \
         with overclocking'), matching this toggle's now off-by-default contract"
    );
}

#[test]
fn coarse_pressure_coupling_enabled_diverges_from_default() {
    let default_off = run(|_sim| {}, 500);
    let forced_on = run(|sim| sim.coarse_pressure_coupling = true, 500);
    assert_ne!(
        default_off, forced_on,
        "coarse_pressure_coupling=true produced byte-identical output to the default uncoupled \
         path in a U-tube scenario over 500 ticks -- either the toggle isn't reaching \
         DrawingSimulation::update's coupling gate site, or this scenario happens not to exercise \
         a tick where coarse.available is true (it should be, at grid 128)"
    );
}

/// The load-bearing claim: OFF is not merely DIFFERENT from ON, it is IDENTICAL to the coarse
/// level never having existed for this tick. `coarse.available` and `coarse_pressure_coupling`
/// gate the identical two call sites (`if self.coarse.available && self.coarse_pressure_coupling`
/// before `coarse_state.tick(...)`, and the same conjunction feeding `settle_tick`'s
/// `coarse_eta`/`coarse_delta` arguments) -- see `coarse_pressure_coupling`'s own doc comment in
/// `sandart-sim/src/lib.rs`. So forcing `coarse.available = false` directly (bypassing the
/// toggle entirely, exercising the SAME code path grid 64 has always taken, which predates this
/// toggle's existence) must produce output bit-identical to leaving `coarse.available` alone and
/// setting the toggle off instead -- whichever of the two conditions is false, both gates read
/// false, and `coarse.available` is the only OTHER field either gate reads (verified: it is used
/// nowhere else in this crate).
#[test]
fn coarse_pressure_coupling_off_matches_coarse_geometry_forced_unavailable() {
    let toggle_off = run(|sim| sim.coarse_pressure_coupling = false, 500);
    let geometry_forced_unavailable = run(
        |sim| {
            // Explicitly enable coarse_pressure_coupling (now off by default) so this exercises
            // the interesting case: coupling WANTS to run, but coarse.available says the coarse
            // level cannot supply anything this tick. coarse.available must be enough on its own
            // to reproduce the toggle-off output regardless of the toggle's own value.
            sim.coarse_pressure_coupling = true;
            sim.coarse.available = false;
        },
        500,
    );
    assert_eq!(
        toggle_off, geometry_forced_unavailable,
        "coarse_pressure_coupling=false must be bit-identical to coarse.available=false (the \
         pre-existing 'coarse level not coupled' signal, e.g. what grid 64 has always produced) \
         -- if this fails, something is reading self.coarse.available or coarse_pressure_coupling \
         independently of the other, rather than the two being a pure OR-gated 'is the coupling \
         off' condition"
    );
}
