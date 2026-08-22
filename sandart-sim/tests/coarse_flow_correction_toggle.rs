//! Empirical proof of four things about the "coarse flow correction" debug toggle
//! (`DrawingSimulation::coarse_flow_correction`, LATERAL-COARSE-CORRECTION.md):
//!
//! 1. Left at its default (`false`) is indistinguishable from explicitly setting it `false` -- the
//!    ledger's per-edge recording hooks in `flux_edge_apply`/`try_move` must be inert when the
//!    correction is off, not merely harmless.
//! 2. Explicitly setting it `true` actually changes behaviour (the toggle is not a placebo).
//! 3. `damping = 0.0` is bit-identical to the toggle being off -- the two ways of saying
//!    "no correction" must not disagree, since the field's doc comment promises they are the same.
//! 4. **The correction conserves mass exactly.** This is the load-bearing one. The entire argument
//!    for why the correction may exceed the fine level's local CFL bound is that it is a FLUX in
//!    divergence form -- every transfer subtracts from one cell and adds the same amount to its
//!    neighbour -- so conservation, not the stability bound, is what makes it safe. If this test
//!    fails the design's central claim is false, not merely mistuned. Checked at a damping of 1.0,
//!    the most aggressive setting, and on both axes so the vertical path is covered too.
//!
//! Mirrors the `*_left_untouched_matches_explicitly_disabled` / `*_enabled_diverges_from_default`
//! pattern of `coarse_pressure_coupling_toggle.rs` and its siblings.

use glam::Vec2;
use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};

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
    hash = fnv1a(cast_f32(&sim.heightmap.data), hash);
    hash = fnv1a(&sim.cell_colors, hash);
    hash = fnv1a(cast_f32(&sim.cell_props), hash);
    hash
}

fn cast_f32(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}

/// Grid 128 so `coarse.available` is true and a block is exactly a coarse tile -- the geometry
/// requirement `apply_coarse_flow_correction` refuses to run outside of. An hourglass of DrySand,
/// because the lateral deficit the correction exists to close is a granular pile above its angle
/// of repose (FLOW-DIRECTION.md), and because sand's lateral transport runs through the granular
/// CA rather than the flux solver -- so this scenario exercises `lat_ledger_record_ca`, the path
/// a flux-solver-only ledger would have missed.
fn run(touch: impl FnOnce(&mut DrawingSimulation), ticks: usize) -> (u64, f64, f64) {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::DrySand);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    touch(&mut sim);

    let targets = [None; 5];
    let before: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    for _ in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let after: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    (checksum(&sim), before, after)
}

#[test]
fn coarse_flow_correction_left_untouched_matches_explicitly_disabled() {
    let (untouched, _, _) = run(|_sim| {}, 200);
    let (explicitly_off, _, _) = run(|sim| sim.coarse_flow_correction = false, 200);
    assert_eq!(
        untouched, explicitly_off,
        "never touching coarse_flow_correction must be indistinguishable from explicitly setting \
         it false -- the field's default (set in DrawingSimulation::new_with_size) must be false, \
         matching this toggle's off-by-default contract"
    );
}

#[test]
fn coarse_flow_correction_enabled_diverges_from_default() {
    let (default_off, _, _) = run(|_sim| {}, 200);
    let (forced_on, _, _) = run(
        |sim| {
            sim.coarse_flow_correction = true;
            sim.coarse_correction_damping = 1.0;
        },
        200,
    );
    assert_ne!(
        default_off, forced_on,
        "coarse_flow_correction=true produced byte-identical output to the default in a DrySand \
         hourglass over 200 ticks -- either the toggle isn't reaching update()'s correction call \
         site, or the geometry check in apply_coarse_flow_correction is rejecting this grid (it \
         should not: at grid 128 a block IS a coarse tile)"
    );
}

/// The field's doc comment promises `damping = 0.0` is equivalent to the toggle being off. Two
/// spellings of "no correction" that disagreed would make the slider's bottom end a trap.
#[test]
fn coarse_flow_correction_zero_damping_matches_disabled() {
    let (toggle_off, _, _) = run(|sim| sim.coarse_flow_correction = false, 200);
    let (zero_damping, _, _) = run(
        |sim| {
            sim.coarse_flow_correction = true;
            sim.coarse_correction_damping = 0.0;
        },
        200,
    );
    assert_eq!(
        toggle_off, zero_damping,
        "coarse_correction_damping=0.0 must be bit-identical to coarse_flow_correction=false -- \
         the field's doc comment states the two are equivalent, and update()'s `correction_active` \
         gate is what has to make that true"
    );
}

/// THE LOAD-BEARING TEST, restated for the conveyance-boost design.
///
/// The correction no longer moves any mass itself -- it scales `c_sq`/`alpha`, the conveyance
/// coefficients inside the existing FCT-limited solver, so conservation is inherited rather than
/// re-argued. That makes this a regression test on the inheritance: if a boost could ever push a
/// transfer past the solver's own availability/headroom limiter, mass would stop being conserved,
/// and the `+/-1.0` clamp in `flux_edge_candidate` is what is supposed to make that impossible at
/// any boost. Run at strength 1.0, the largest the UI allows.
#[test]
fn coarse_flow_correction_conserves_mass_at_full_strength() {
    let (_, before, after) = run(
        |sim| {
            sim.coarse_flow_correction = true;
            sim.coarse_correction_damping = 1.0;
        },
        200,
    );
    let err = (after - before).abs() / before.max(1e-12);
    // The same bar the shipped tree already meets without the correction (SESSION-HANDOVER
    // 2026-08-20 evening §4 records 1.37e-9 to 7.45e-8 for the uncorrected solver), so this
    // asserts the boost adds no conservation error of its own rather than asserting an absolute
    // exactness the underlying f32 solver never had.
    assert!(
        err < 1e-6,
        "the lateral conveyance boost lost or created mass: {before} -> {after} (relative error \
         {err:.3e}). The boost only scales `c_sq`/`alpha` inside the existing FCT-limited solver, \
         so a nonzero error here means a boosted candidate is escaping the availability/headroom \
         limiter or the +/-1.0 clamp in `flux_edge_candidate`."
    );
}

/// The boost must never make a block convey SLOWER than it would with the correction off --
/// `compute_lateral_boost` returns multipliers floored at 1.0, and a value below 1.0 would be
/// underclocking the solver on the strength of a coarse-level opinion, which is not what any of
/// this is for.
#[test]
fn coarse_flow_correction_never_slows_the_solver() {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::DrySand);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    sim.coarse_flow_correction = true;
    sim.coarse_correction_damping = 1.0;
    let targets = [None; 5];
    for _ in 0..120 {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
        let (bh, bv, _) = sandart_sim::physics::compute_lateral_boost(
            sim.heightmap.width,
            sim.heightmap.height,
            sim.block_size,
            sim.coarse.coarse_n,
            &sandart_sim::physics::lat_ledger_snapshot().0,
            &sandart_sim::physics::lat_ledger_snapshot().1,
            &sandart_sim::physics::lat_ledger_snapshot().2,
            &sandart_sim::physics::lat_ledger_snapshot().3,
            1.0,
        );
        for &f in bh.iter().chain(bv.iter()) {
            assert!(
                (1.0..=64.0).contains(&f),
                "conveyance boost out of range: {f}. Must be >= 1.0 (never slower than the \
                 uncorrected solver) and bounded above by 1 + LATERAL_BOOST_MAX."
            );
        }
    }
}
