//! Empirical proof that `DrawingSimulation::pressure_heatmap_head_field` (task 2.32's pressure
//! heat-map source selector) is a true no-op on the SIMULATION regardless of its value -- unlike
//! `fresh_pressure_field` (see `fresh_pressure_field_toggle.rs`, the
//! precedent this file's structure was copied from), which really do change `settle_tick`'s
//! behaviour when enabled, this toggle changes ONLY what `DrawingSimulation::pressure_field_texels`
//! returns to a caller that asks for it. Nothing in `settle_tick`/`update` ever reads this field,
//! so `true` and `false` must produce byte-identical simulation output, not just "the same output
//! when left at its default" -- the stronger claim `fresh_pressure_field_toggle.rs`'s own
//! `_left_untouched_matches_explicitly_disabled` test makes is the right shape, but this toggle
//! additionally needs "explicitly true == explicitly false" since divergence is never expected.
//!
//! A genuine integration test (lives in `tests/`, its own compilation unit), exercising only the
//! same public surface any other consumer (sandart-wasm, the desktop app) would use -- proves
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

/// Runs a 200-tick Hourglass Sand-fall-under-Water sequence (gravity active), touching
/// `pressure_heatmap_head_field` exactly as instructed by `touch`. Same scenario
/// `fresh_pressure_field_toggle.rs` uses, for the same reason: a deep, continuously-fed liquid
/// column under a narrow neck gives any accidental read of this field (were one to exist) the
/// most ticks to show up in. Deliberately never calls `pressure_field_texels` itself -- this test
/// asserts the toggle doesn't perturb `update`, not anything about the heat-map's own output.
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
fn pressure_heatmap_head_field_left_untouched_matches_explicitly_disabled() {
    let untouched = run(|_sim| {});
    let explicitly_off = run(|sim| sim.pressure_heatmap_head_field = false);
    assert_eq!(
        untouched, explicitly_off,
        "never setting pressure_heatmap_head_field must be indistinguishable from explicitly \
         setting it to false -- if this fails, something outside pressure_field_texels is \
         reading the field's default in a way that affects simulation output"
    );
}

#[test]
fn pressure_heatmap_head_field_enabled_does_not_perturb_simulation() {
    // Unlike `fresh_pressure_field_enabled_diverges_from_default` (its namesake in
    // `fresh_pressure_field_toggle.rs`), the expectation here is the OPPOSITE: this toggle is
    // visualisation-only (see its doc comment in sandart-sim/src/lib.rs and the task-2.32 brief's
    // "Do NOT wire the head field into settle_tick or update's physics" constraint), so turning
    // it on must be a complete no-op on `update`'s output -- `compute_head_field` is only ever
    // invoked from inside `pressure_field_texels`, which this test never calls.
    let default_off = run(|_sim| {});
    let forced_on = run(|sim| sim.pressure_heatmap_head_field = true);
    assert_eq!(
        default_off, forced_on,
        "pressure_heatmap_head_field=true produced simulation output that diverged from the \
         default -- this field must be read ONLY by pressure_field_texels; if this fails, it (or \
         something it gates) has leaked into settle_tick/update, which the task-2.32 brief \
         explicitly forbids (visualisation only, no transport, no mass moved)."
    );
}
