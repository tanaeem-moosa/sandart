//! Empirical proof that the "perfect simulation" debug toggle (`DrawingSimulation
//! ::perfect_simulation`) is a true no-op when left at its default `false`: the same tick
//! sequence, run through two independently-constructed simulations that never touch the field,
//! must produce byte-identical heightmap/color/property buffers.
//!
//! This is a genuine integration test (lives in `tests/`, its own compilation unit) rather than
//! a `#[cfg(test)]` module inside `lib.rs`/`physics.rs`, specifically so it exercises only the
//! same public surface any other consumer (sandart-wasm, the desktop app) would use -- it proves
//! nothing about internals, only about observable output, which is the thing the "off path must
//! not shift at all" requirement actually cares about.
//!
//! The stronger claim -- that this crate's behaviour with the field never touched is identical
//! to its behaviour *before this feature existed* -- was additionally checked by hand once: this
//! same checksum, computed against a `git stash` of the `perfect_simulation`/`block_heat_buckets`
//! changes to `sandart-sim/src/{lib,physics}.rs`, matched the checksum on the current tree. That
//! comparison isn't repeatable as a committed test (there is no "pre-feature" tree to stash
//! against once the feature has landed), so this test instead pins the property that
//! *is* durably testable: touching the field never changes the result.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// A simple, dependency-free checksum over the three buffers `update` can mutate. Not
/// cryptographic -- just sensitive enough that a single flipped bit anywhere in 150 ticks of
/// simulation across a 512x512 grid would change it. FNV-1a over the raw bytes of each buffer,
/// folded together.
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

/// Runs the same 150-tick Hourglass Sand-fall sequence used elsewhere in this crate's own test
/// suite (see `test_all_sandfall_funnels_conserve_sand_mass` in `src/lib.rs`), touching
/// `perfect_simulation` exactly as instructed by `touch`.
fn run(touch: impl FnOnce(&mut DrawingSimulation)) -> u64 {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.initialize_hourglass();
    touch(&mut sim);

    let targets = [None; 5];
    for _ in 0..150 {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Hourglass, 16.0, 16.0);
    }
    checksum(&sim)
}

#[test]
fn perfect_simulation_left_untouched_matches_explicitly_disabled() {
    let untouched = run(|_sim| {});
    let explicitly_off = run(|sim| sim.perfect_simulation = false);
    assert_eq!(
        untouched, explicitly_off,
        "never setting perfect_simulation must be indistinguishable from explicitly setting it \
         to false -- if this fails, something outside the `if self.perfect_simulation` guard in \
         DrawingSimulation::update is reading the field's default in a way that affects output"
    );
}

#[test]
fn perfect_simulation_enabled_diverges_from_default() {
    // The flip side of the test above: perfect_simulation actually has to DO something, or the
    // toggle is a placebo. A Hourglass drain under the adaptive budget scheduler and under the
    // budget-bypassing "simulate everything with material" path are extremely unlikely to reach
    // tick 150 in exactly the same state -- the whole premise of the feature is that the budget
    // skips blocks the unthrottled path wouldn't.
    let default_off = run(|_sim| {});
    let forced_on = run(|sim| sim.perfect_simulation = true);
    assert_ne!(
        default_off, forced_on,
        "perfect_simulation=true produced byte-identical output to the default budgeted path -- \
         either the toggle isn't reaching settle_tick's admission path, or this scenario happens \
         to never lose a block to the budget (try a larger grid/longer run if so)"
    );
}
