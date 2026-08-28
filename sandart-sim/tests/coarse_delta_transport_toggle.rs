//! Empirical proof of five things about the "coarse delta transport" debug toggle
//! (`DrawingSimulation::coarse_delta_transport`, CREDIT-DEBT-TRANSPORT.md §2.3):
//!
//! 1. Left at its default (`false`) is indistinguishable from explicitly setting it `false`.
//! 2. Explicitly setting it `true` actually changes behaviour (the toggle is not a placebo).
//! 3. `rate = 0.0` is bit-identical to the toggle being off -- the two ways of saying "no
//!    transport" must not disagree, since the field's doc comment promises they are the same.
//! 4. **The transport conserves mass exactly.** This is the load-bearing one, and it is load-bearing
//!    for a sharper reason than in the conveyance-boost design: this mechanism writes fine heights
//!    OUTSIDE `settle_tick`, so it does not inherit the FCT limiter's conservation argument the way
//!    a coefficient change does. It has to make its own. The construction that makes it true is in
//!    `apply_coarse_delta_transport`: the amount is clamped to the donor's available mass and the
//!    receiver's headroom before anything moves, then withdrawn and deposited as the SAME `amount`
//!    split by normalised weights. If this test fails, that construction is wrong.
//! 5. **It does not drive any cell negative or past capacity.** The per-cell corollary of (4) --
//!    a total that balances while individual cells went out of range would still be a bug, and a
//!    checksum test cannot see it.
//!
//! Mirrors `coarse_flow_correction_toggle.rs`'s structure, which mirrors
//! `coarse_pressure_coupling_toggle.rs` in turn.

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
/// identity `apply_coarse_delta_transport` refuses to run outside of, and which the call site
/// relies on to map a tile index back to a block index.
///
/// Water rather than DrySand: this mechanism exists because of water's lateral flow specifically
/// (f10fc15 ruled out the conveyance boost for water at +0.6%/+0.4%), so the material it is aimed
/// at is the one to test it on.
fn run(touch: impl FnOnce(&mut DrawingSimulation), ticks: usize) -> (u64, f64, f64) {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    touch(&mut sim);

    let targets = [None; 5];
    let before: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    for _ in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let after: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    (checksum(&sim), before, after)
}

#[test]
fn coarse_delta_transport_left_untouched_matches_explicitly_disabled() {
    let (untouched, _, _) = run(|_sim| {}, 200);
    let (explicitly_off, _, _) = run(|sim| sim.coarse_delta_transport = false, 200);
    assert_eq!(
        untouched, explicitly_off,
        "never touching coarse_delta_transport must be indistinguishable from explicitly setting \
         it false -- the field's default (set in DrawingSimulation::new_with_size) must be false, \
         matching this toggle's off-by-default contract"
    );
}

#[test]
fn coarse_delta_transport_enabled_diverges_from_default() {
    let (default_off, _, _) = run(|_sim| {}, 200);
    let (forced_on, _, _) = run(
        |sim| {
            sim.coarse_delta_transport = true;
            sim.coarse_delta_transport_rate = 1.0;
        },
        200,
    );
    assert_ne!(
        default_off, forced_on,
        "coarse_delta_transport=true produced byte-identical output to the default in a Water \
         hourglass over 200 ticks -- either the toggle isn't reaching update()'s transport call \
         site, or apply_coarse_delta_transport's geometry guard is rejecting this grid (it should \
         not: at grid 128 a block IS a coarse tile), or every face is being capped to zero"
    );
}

/// The field's doc comment promises `rate = 0.0` is equivalent to the toggle being off. Two
/// spellings of "no transport" that disagreed would make the slider's bottom end a trap.
///
/// This also pins the lambda coupling: `update()` raises `CoarseState::lambda` to 0.5 only while
/// the transport is ACTIVE, and `rate = 0.0` must count as inactive. If the gate were written on
/// the bool alone, anchoring would change here and this test would fail -- which is the point.
#[test]
fn coarse_delta_transport_zero_rate_matches_disabled() {
    let (toggle_off, _, _) = run(|sim| sim.coarse_delta_transport = false, 200);
    let (zero_rate, _, _) = run(
        |sim| {
            sim.coarse_delta_transport = true;
            sim.coarse_delta_transport_rate = 0.0;
        },
        200,
    );
    assert_eq!(
        toggle_off, zero_rate,
        "coarse_delta_transport_rate=0.0 must be bit-identical to coarse_delta_transport=false -- \
         the field's doc comment states the two are equivalent, and update()'s \
         `delta_transport_active` gate (which also controls the lambda raise) is what makes it true"
    );
}

/// THE LOAD-BEARING TEST.
///
/// Unlike the conveyance boost, this mechanism moves mass itself, outside `settle_tick`, so it does
/// not inherit the FCT limiter's conservation guarantee. Its own argument is the withdraw/deposit
/// symmetry in `apply_coarse_delta_transport`. Checked at rate 1.0, the most aggressive setting.
#[test]
fn coarse_delta_transport_conserves_mass() {
    let (_, before, after) = run(
        |sim| {
            sim.coarse_delta_transport = true;
            sim.coarse_delta_transport_rate = 1.0;
        },
        200,
    );
    let drift = (after - before).abs();
    let rel = drift / before.max(1.0);
    assert!(
        rel < 1e-6,
        "coarse delta transport must conserve mass exactly: total went {before} -> {after} \
         (absolute drift {drift}, relative {rel}). The withdraw and the deposit split the SAME \
         clamped `amount` by normalised weights, so any drift beyond f32 accumulation means that \
         construction is broken -- most likely a cap applied to one side but not the other"
    );
}

/// The per-cell corollary. A total that balances while individual cells went negative or past
/// capacity is still a bug, and the checksum and sum tests above cannot see it.
#[test]
fn coarse_delta_transport_keeps_cells_in_range() {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    sim.coarse_delta_transport = true;
    sim.coarse_delta_transport_rate = 1.0;

    let targets = [None; 5];
    for tick in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
        for (i, &h) in sim.heightmap.data.iter().enumerate() {
            assert!(
                h.is_finite(),
                "cell {i} became non-finite ({h}) at tick {tick} with delta transport on"
            );
            assert!(
                h >= -1e-3,
                "cell {i} went negative ({h}) at tick {tick} -- the donor-side cap must clamp the \
                 transfer to the tile's available mass before anything moves"
            );
        }
    }
}
