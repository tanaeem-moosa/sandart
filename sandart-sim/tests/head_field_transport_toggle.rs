//! Empirical proof that the "drive transport from the head field" debug toggle
//! (`DrawingSimulation::head_field_transport`, task #55 step 3) is a true no-op when left at its
//! default `false`, and that it is NOT a no-op when turned on over a liquid scenario -- mirrors
//! `fresh_pressure_field_toggle.rs`, the precedent this toggle's plumbing (sim field ->
//! `physics::settle_tick` parameter -> `sandart-wasm/src/lib.rs` setter -> `demo.js` -> a checkbox
//! in `index.html`) was copied from end to end.
//!
//! This is a genuine integration test (lives in `tests/`, its own compilation unit) rather than a
//! `#[cfg(test)]` module inside `lib.rs`/`physics.rs`, specifically so it exercises only the same
//! public surface any other consumer (sandart-wasm, the desktop app) would use -- it proves
//! nothing about internals, only about observable output.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// A simple, dependency-free checksum over the three buffers `update` can mutate. Not
/// cryptographic -- just sensitive enough that a single flipped bit anywhere in 150+ ticks of
/// simulation would change it. FNV-1a over the raw bytes of each buffer, folded together. Copied
/// verbatim from `fresh_pressure_field_toggle.rs` (deliberately duplicated, not shared, so each
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

/// Runs a 200-tick Hourglass Sand-fall-under-Water sequence (gravity active, material Water --
/// wholly liquid, so both endpoints of nearly every wet edge clear this toggle's `liquidity >=
/// LIQUID_ELLIPTIC_THRESHOLD` gate), touching `head_field_transport` exactly as instructed by
/// `touch`. Same scenario `fresh_pressure_field_toggle.rs` uses, for the same reason: a deep,
/// continuously-fed liquid column under a narrow neck gives the toggle the most ticks to diverge
/// on if it does anything at all.
///
/// `sim.update`'s own `_material` parameter is unused (prefixed `_` in its signature) -- passing
/// `MaterialMode::Water` there does NOT actually set any cell's wetness, unlike what
/// `fresh_pressure_field_toggle.rs`'s identical-looking call might suggest (that toggle's own
/// branch is not liquidity-gated, so it never mattered there). This toggle's branch IS
/// liquidity-gated, so the material must be applied for real via `apply_preset`, which writes
/// `PROP_WETNESS` into every cell -- otherwise the whole scenario is silently DrySand (wetness
/// 0.00, `new_with_size`'s own default), no cell ever clears `LIQUID_ELLIPTIC_THRESHOLD`, and the
/// enabled/disabled runs are trivially identical for the wrong reason (confirmed: this is exactly
/// what a first draft of this test measured before this fix).
fn run(touch: impl FnOnce(&mut DrawingSimulation)) -> u64 {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.initialize_hourglass();
    sim.apply_preset(MaterialMode::Water);
    touch(&mut sim);

    let targets = [None; 5];
    for _ in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
    }
    checksum(&sim)
}

#[test]
fn head_field_transport_left_untouched_matches_explicitly_disabled() {
    let untouched = run(|_sim| {});
    let explicitly_off = run(|sim| sim.head_field_transport = false);
    assert_eq!(
        untouched, explicitly_off,
        "never setting head_field_transport must be indistinguishable from explicitly setting it \
         to false -- if this fails, something outside the `if head_field_transport` guards in \
         physics::settle_tick is reading the field's default in a way that affects output"
    );
}

#[test]
fn head_field_transport_enabled_diverges_from_default() {
    // The flip side of the test above: head_field_transport actually has to DO something over a
    // liquid scenario, or the toggle is a placebo. Swapping the local-fill-plus-overburden
    // driving head for the unified hydraulic head field on every liquid edge is documented (see
    // `head_field_transport`'s and `settle_tick`'s doc comments) to change the driving head's
    // exact value on both the lateral and vertical (gravity-aligned) edge solvers.
    let default_off = run(|_sim| {});
    let forced_on = run(|sim| sim.head_field_transport = true);
    assert_ne!(
        default_off, forced_on,
        "head_field_transport=true produced byte-identical output to the default driving-head \
         path -- either the toggle isn't reaching settle_tick's lateral/vertical edge solvers, or \
         this scenario happens not to exercise a liquid edge with both endpoints above \
         LIQUID_ELLIPTIC_THRESHOLD (try a longer run or a wetter scenario if so)"
    );
}
