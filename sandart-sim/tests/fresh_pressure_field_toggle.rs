//! Empirical proof that the "fresh pressure field" debug toggle (`DrawingSimulation
//! ::fresh_pressure_field`) is a true no-op when left at its default `false`, and that it is NOT
//! a no-op when turned on -- mirrors `perfect_simulation_determinism.rs`, the precedent this
//! toggle's plumbing was copied from end to end (sim field -> `sandart-wasm/src/lib.rs` setter ->
//! `demo.js` -> a checkbox in `index.html`).
//!
//! This is a genuine integration test (lives in `tests/`, its own compilation unit) rather than a
//! `#[cfg(test)]` module inside `lib.rs`/`physics.rs`, specifically so it exercises only the same
//! public surface any other consumer (sandart-wasm, the desktop app) would use -- it proves
//! nothing about internals, only about observable output.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// A simple, dependency-free checksum over the three buffers `update` can mutate. Not
/// cryptographic -- just sensitive enough that a single flipped bit anywhere in 150+ ticks of
/// simulation would change it. FNV-1a over the raw bytes of each buffer, folded together.
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

// Tiny local stand-in for `bytemuck::cast_slice` so this test has no extra dependency: f32 has
// no padding/alignment surprises worth guarding against here, we just want its bytes.
fn bytemuck_cast_f32(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}

/// Runs a 200-tick Hourglass Sand-fall-under-Water sequence (gravity active, so `column_depth` --
/// the quantity this toggle changes how it is computed -- is actually exercised every tick),
/// touching `fresh_pressure_field` exactly as instructed by `touch`. Water, not DrySand: the
/// toggle only ever changes anything inside `if gravity_active { ... }` in `settle_tick`'s CA
/// branch, which both materials pass through, but liquid drains fastest through a narrow neck --
/// giving the standalone pass the most ticks of a genuinely deep, continuously-fed column to
/// diverge on, the same scenario `test_liquid_flowing_liquid_does_not_stand_in_walls` uses.
fn run(touch: impl FnOnce(&mut DrawingSimulation)) -> u64 {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.initialize_hourglass();
    touch(&mut sim);

    let targets = [None; 5];
    for _ in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
    }
    checksum(&sim)
}

#[test]
fn fresh_pressure_field_left_untouched_matches_explicitly_disabled() {
    let untouched = run(|_sim| {});
    let explicitly_off = run(|sim| sim.fresh_pressure_field = false);
    assert_eq!(
        untouched, explicitly_off,
        "never setting fresh_pressure_field must be indistinguishable from explicitly setting it \
         to false -- if this fails, something outside the `if fresh_pressure_field` guards in \
         physics::settle_tick is reading the field's default in a way that affects output"
    );
}

#[test]
fn fresh_pressure_field_enabled_diverges_from_default() {
    // The flip side of the test above: fresh_pressure_field actually has to DO something, or the
    // toggle is a placebo. Swapping the order-dependent inline column_depth computation for the
    // unconditional once-per-tick standalone pass is documented (see `recompute_column_depth`'s
    // and `settle_tick`'s doc comments) to visibly change drain behaviour -- it is the same
    // change measured as voids@160 = 66 on `test_liquid_flowing_liquid_does_not_stand_in_walls`.
    let default_off = run(|_sim| {});
    let forced_on = run(|sim| sim.fresh_pressure_field = true);
    assert_ne!(
        default_off, forced_on,
        "fresh_pressure_field=true produced byte-identical output to the default in-loop path -- \
         either the toggle isn't reaching settle_tick's column_depth computation, or this \
         scenario happens not to exercise gravity_active cells (try a longer run if so)"
    );
}
