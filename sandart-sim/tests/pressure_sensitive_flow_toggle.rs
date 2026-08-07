//! Empirical proof that the "pressure-sensitive flow rate" debug toggle
//! (`DrawingSimulation::pressure_sensitive_flow`, task #63) is a true no-op when left at its
//! default `false`, and that it is NOT a no-op when turned on over a liquid scenario -- mirrors
//! `head_field_transport_toggle.rs` / `fresh_pressure_field_toggle.rs`, the precedent this
//! toggle's plumbing (sim field -> `physics::settle_tick` parameter -> `sandart-wasm/src/lib.rs`
//! setter -> `demo.js` -> a checkbox in `index.html`) was copied from end to end.
//!
//! This is a genuine integration test (lives in `tests/`, its own compilation unit) rather than a
//! `#[cfg(test)]` module inside `lib.rs`/`physics.rs`, specifically so it exercises only the same
//! public surface any other consumer (sandart-wasm, the desktop app) would use -- it proves
//! nothing about internals, only about observable output.
//!
//! The third check here has no counterpart in the two files above and is the one that matters for
//! this particular toggle: `pressure_rate_factor` is capped at `1.0`, so turning this on may only
//! ever REMOVE flux, never add any. That is asserted against measured moved mass rather than
//! inferred from the constant's definition.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// A simple, dependency-free checksum over the three buffers `update` can mutate. Not
/// cryptographic -- just sensitive enough that a single flipped bit anywhere in 200 ticks of
/// simulation would change it. FNV-1a over the raw bytes of each buffer, folded together. Copied
/// verbatim from `head_field_transport_toggle.rs` (deliberately duplicated, not shared, so each
/// integration test file stays a fully independent compilation unit).
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

/// Runs `ticks` ticks of an Hourglass water sequence, touching `pressure_sensitive_flow` exactly
/// as instructed by `touch`. Returns a per-tick trace: `(checksum after this tick, cumulative
/// absolute height change from the initial state after this tick)`.
///
/// A TRACE rather than a single end value, because the only-ever-slower property below can only be
/// compared while both runs still share an input state -- see that test's own comment.
///
/// `apply_preset(MaterialMode::Water)` is what actually writes `PROP_WETNESS` into every cell;
/// `update`'s own `_material` parameter is unused. Without it the whole scenario is silently
/// DrySand, no cell clears `LIQUID_ELLIPTIC_THRESHOLD`, and every run here would be identical for
/// the wrong reason (the exact trap `head_field_transport_toggle.rs` records falling into).
fn run(ticks: usize, touch: impl FnOnce(&mut DrawingSimulation)) -> Vec<(u64, f64)> {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.initialize_hourglass();
    sim.apply_preset(MaterialMode::Water);
    touch(&mut sim);

    let initial = sim.heightmap.data.clone();
    let targets = [None; 5];
    let mut trace = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
        let displaced: f64 = sim
            .heightmap
            .data
            .iter()
            .zip(initial.iter())
            .map(|(a, b)| (a - b).abs() as f64)
            .sum();
        trace.push((checksum(&sim), displaced));
    }
    trace
}

#[test]
fn pressure_sensitive_flow_left_untouched_matches_explicitly_disabled() {
    let untouched = run(200, |_sim| {});
    let explicitly_off = run(200, |sim| sim.pressure_sensitive_flow = false);
    assert_eq!(
        untouched.last().unwrap().0,
        explicitly_off.last().unwrap().0,
        "never setting pressure_sensitive_flow must be indistinguishable from explicitly setting \
         it to false -- if this fails, something outside the `if pressure_sensitive_flow` guards \
         in physics::settle_tick is reading the field's default in a way that affects output"
    );
}

#[test]
fn pressure_sensitive_flow_enabled_diverges_from_default() {
    // The flip side of the test above: the toggle actually has to DO something over a liquid
    // scenario, or it is a placebo. An Hourglass full of water develops a free surface and a
    // draining neck, both of which are exactly the sub-one-cell-of-head donors
    // `pressure_rate_factor` attenuates.
    let default_off = run(200, |_sim| {});
    let forced_on = run(200, |sim| sim.pressure_sensitive_flow = true);
    assert_ne!(
        default_off.last().unwrap().0,
        forced_on.last().unwrap().0,
        "pressure_sensitive_flow=true produced byte-identical output to the default rate path -- \
         either the toggle isn't reaching settle_tick's lateral/vertical edge sites, or every \
         liquid donor in this scenario already carries a full cell of head (where the factor is \
         exactly 1.0 by design), in which case this needs a shallower or longer scenario"
    );
}

#[test]
fn pressure_sensitive_flow_can_only_remove_flux_never_add() {
    // The safety property the whole design rests on: `pressure_rate_factor` returns at most `1.0`,
    // so every edge it touches conveys at most what it would have conveyed with the toggle off,
    // and free-falling material (factor exactly 1.0) conveys exactly what it would have. This is
    // what makes the toggle safe against `spec_transport_speed_is_bounded` and against the CFL
    // bound `wave_params`' `c_sq` was chosen to respect, without measuring either: the change
    // cannot push any edge past a limit it was already inside.
    //
    // MEASURED AT THE FIRST DIVERGENT TICK, and the window matters. The property is per-edge and
    // per-tick ("given the same input state, this edge moves no more mass"), so it is only
    // testable while both runs still SHARE an input state. Every tick before the first divergence
    // is bit-identical in both runs, so cumulative displacement up to and including that tick
    // isolates exactly one tick's worth of difference, produced from identical inputs.
    //
    // From the NEXT tick onward they are two different simulations and total displacement stops
    // being monotone in per-edge rate: slowing an edge changes which cells hold material later,
    // which changes which blocks the adaptive scheduler wakes, and either can raise the total.
    // MEASURED, so nobody re-derives it the hard way: over the full 200 ticks this same scenario
    // gives 199.5935 with the toggle ON against 199.5251 with it off -- ON is HIGHER, by 0.03%.
    // That is trajectory divergence, not a violated bound. Do NOT "fix" this test by comparing at
    // tick 200; that comparison does not test this property at all.
    let off = run(200, |_sim| {});
    let on = run(200, |sim| sim.pressure_sensitive_flow = true);
    let first_diff = off
        .iter()
        .zip(on.iter())
        .position(|((a, _), (b, _))| a != b)
        .expect(
            "the toggle changed nothing in 200 ticks, so the inequality below would hold \
             vacuously -- covered by pressure_sensitive_flow_enabled_diverges_from_default, so if \
             that test is also failing, fix it first",
        );
    let (_, off_displaced) = off[first_diff];
    let (_, on_displaced) = on[first_diff];
    assert!(
        on_displaced <= off_displaced + 1e-6,
        "at tick {} -- the FIRST tick where the two runs differ, so both produced it from an \
         identical state -- pressure_sensitive_flow=true displaced MORE mass ({on_displaced:.6}) \
         than the default path ({off_displaced:.6}). The rate factor is capped at 1.0, so this \
         means either the cap was removed or the factor is being applied somewhere it multiplies \
         a different quantity.",
        first_diff + 1
    );
}

/// What turning this toggle on actually COSTS per tick at production resolution, since it is the
/// third condition that makes the head field advance (`settle_tick`'s `head_field_needs_advance`)
/// and is therefore not free even though it moves no mass of its own. DIAGNOSTIC (reproduce-only,
/// no assertions -- a wall-clock number does not belong in a pass/fail gate). Run with
/// `--ignored --nocapture`.
#[test]
#[ignore]
fn diag_pressure_sensitive_flow_cost_w512() {
    const TICKS: usize = 200;
    let measure = |on: bool, advance_only: bool| -> f64 {
        let mut sim = DrawingSimulation::new_with_size(512);
        sim.sandbox_shape = SandboxShape::Hourglass;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.initialize_hourglass();
        sim.apply_preset(MaterialMode::Water);
        sim.pressure_sensitive_flow = on;
        // Middle arm: advance the head field WITHOUT the rate change, by borrowing the heat-map
        // overlay's own advance condition. Isolates "what does keeping the field live cost" from
        // "what does attenuating the rate cost".
        sim.pressure_heatmap_head_field = advance_only;
        let targets = [None; 5];
        // One untimed tick first, so buffer growth and first-touch page faults land outside the
        // measurement window rather than inside the "off" arm only.
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
        let start = std::time::Instant::now();
        for _ in 0..TICKS {
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
        }
        start.elapsed().as_secs_f64() * 1000.0 / TICKS as f64
    };
    let off = measure(false, false);
    let field_only = measure(false, true);
    let on = measure(true, false);
    println!(
        "diag_pressure_sensitive_flow_cost_w512: {TICKS} ticks at w=512, Hourglass/Water\n  \
         off              = {off:.3} ms/tick\n  \
         head field only  = {field_only:.3} ms/tick  ({:+.1}%)\n  \
         rate toggle on   = {on:.3} ms/tick  ({:+.1}%)",
        100.0 * (field_only - off) / off,
        100.0 * (on - off) / off
    );
}
