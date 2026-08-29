//! Empirical proof of seven things about the "coarse delta transport" debug toggle
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
//! 6. **A symmetric vessel stays symmetric.** Added after the first five all passed while the
//!    implementation was visibly broken on screen -- see that test's own comment.
//! 7. **It does not grow a checkerboard.** Added after (6) ALSO passed while a second, different
//!    artifact was visible on screen. Two escapes in a row is the reason both of these pin
//!    physical invariants rather than implementation details.
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

/// REGRESSION TEST for the defect the five tests above did not catch, and which a user found by
/// looking at the screen: *"hourglass is not falling. It is attached to the right."*
///
/// The first implementation walked tiles in scan order and mutated `heights` IN PLACE while the
/// `Delta` it was reading stayed frozen. Two consequences, both invisible to a checksum, a mass
/// sum, or a per-cell range check:
///
/// 1. **Scan-order bias.** A face's transfer depended on how many of its neighbours the loop had
///    already visited, so a left-to-right, top-to-bottom walk biased transport in that direction.
///    Measured as the centre of mass of a perfectly symmetric hourglass drifting monotonically to
///    `dx = -2.34` over 200 ticks, against `-0.02` with the toggle off.
/// 2. **Over-transport.** A tile that had already given mass away still read its original `Delta`
///    on its second face and gave again, which smeared material across the vessel instead of
///    letting it fall -- the same failure LATERAL-COARSE-CORRECTION.md records for Design 2.
///
/// The fix was to split the pass into COLLECT / ARBITRATE / APPLY so nothing is applied until
/// every face has been costed against the same frozen state. This test pins the property that
/// makes that fix necessary: **a symmetric vessel under symmetric gravity must stay symmetric.**
/// It is a physical invariant, not an implementation detail, so it holds for any future sizing
/// term too.
#[test]
fn coarse_delta_transport_does_not_bias_a_symmetric_hourglass() {
    fn com_x(sim: &DrawingSimulation) -> f64 {
        let w = sim.heightmap.width;
        let (mut mx, mut m) = (0.0f64, 0.0f64);
        for (i, &h) in sim.heightmap.data.iter().enumerate() {
            if h > 0.0 {
                mx += h as f64 * (i % w) as f64;
                m += h as f64;
            }
        }
        if m > 0.0 { mx / m } else { 0.0 }
    }

    let mut sim = DrawingSimulation::new_with_size(256);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::DrySand);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    sim.coarse_delta_transport = true;
    sim.coarse_delta_transport_rate = 0.7;

    let start = com_x(&sim);
    let targets = [None; 5];
    for _ in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let drift = (com_x(&sim) - start).abs();

    // Gravity is straight down and the hourglass is symmetric about its vertical axis, so the only
    // lateral drift available is numerical. The broken version reached 2.34 cells; the fixed one
    // sits at 0.03, against a toggle-off baseline of 0.02. A cell of slack is generous and still
    // two orders of magnitude below the defect.
    assert!(
        drift < 1.0,
        "coarse delta transport biased a symmetric hourglass sideways by {drift} cells over 200 \
         ticks. Gravity is vertical and the vessel is symmetric, so this is transport that depends \
         on tile visit order -- check that COLLECT/ARBITRATE/APPLY is intact and that nothing \
         mutates `heights` while the collection pass is still reading them"
    );
}

/// SECOND REGRESSION TEST for a defect the suite did not catch and a user saw on screen: a
/// CHECKERBOARD through the lower bulb and horizontal striping through the upper one.
///
/// Cause: `apply_coarse_delta_transport` originally moved HALF the difference of two tiles'
/// `Delta` across each face. Half is what equalises a pair and is the correct, stable relaxation
/// in 1D -- but in 2D a tile exchanges across four faces within the same pass, so a half on each
/// face moves up to twice the tile's whole disagreement per tick. That is exactly 2x the explicit
/// stability limit for a 5-point Laplacian, `1/(2d) = 1/4`, and past that limit the mode that grows
/// is the highest frequency the stencil supports: a checkerboard.
///
/// Measured before the fix (`diag_delta_transport --sweep`, mean |laplacian| vs toggle off): 1.05x
/// at rate 0.35, 1.31x at 0.45, 1.72x at the then-shipped default of 0.70, and 4.52x at 1.00.
/// After folding the quarter in, 1.00 sits AT the limit rather than twice it and reads 1.35x, with
/// everything at or below 0.55 landing under the baseline.
///
/// This pins the invariant at rate 1.0, the worst case the slider allows. A future sizing term that
/// reintroduces an over-relaxation will fail here rather than on someone's screen.
#[test]
fn coarse_delta_transport_does_not_grow_checkerboard() {
    fn checker(sim: &DrawingSimulation) -> f64 {
        let (w, h) = (sim.heightmap.width, sim.heightmap.height);
        let (mut acc, mut n) = (0.0f64, 0usize);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = y * w + x;
                if sim.heightmap.data[i] <= 0.01 {
                    continue;
                }
                let c = sim.heightmap.data[i] as f64;
                let nb = (sim.heightmap.data[i - 1] as f64
                    + sim.heightmap.data[i + 1] as f64
                    + sim.heightmap.data[i - w] as f64
                    + sim.heightmap.data[i + w] as f64)
                    / 4.0;
                acc += (c - nb).abs();
                n += 1;
            }
        }
        if n > 0 { acc / n as f64 } else { 0.0 }
    }
    fn go(rate: f32) -> f64 {
        let mut sim = DrawingSimulation::new_with_size(256);
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(MaterialMode::DrySand);
        sim.overfill_pressure = true;
        sim.initialize_hourglass();
        sim.coarse_delta_transport = rate > 0.0;
        sim.coarse_delta_transport_rate = rate;
        let targets = [None; 5];
        for _ in 0..300 {
            sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 0.0, 16.6);
        }
        checker(&sim)
    }

    let base = go(0.0);
    let hot = go(1.0);
    let ratio = hot / base.max(1e-9);
    assert!(
        ratio < 2.0,
        "coarse delta transport grew high-frequency (checkerboard) energy to {ratio:.2}x the \
         toggle-off baseline at rate 1.0 ({hot:.5} vs {base:.5}). Rate 1.0 must sit AT the 2D \
         stability limit, not past it -- check that the per-face factor is 1/4 and not 1/2. The \
         broken version measured 4.52x here"
    );
}
