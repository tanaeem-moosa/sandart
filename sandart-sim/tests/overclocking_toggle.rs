//! Empirical proof that the "overclocking" (multi-rate block scheduler) debug toggle
//! (`DrawingSimulation::overclocking_enabled`, OVERCLOCKING.md) is a true no-op when left at its
//! default `false`, that it is NOT a no-op when turned on over a liquid scenario, and that mass
//! conservation -- the one thing OVERCLOCKING.md calls non-negotiable -- holds with it on. Mirrors
//! `pressure_sensitive_flow_toggle.rs`'s `*_left_untouched_matches_explicitly_disabled` /
//! `*_enabled_diverges_from_default` pattern, the precedent every other debug toggle in this
//! codebase follows.
//!
//! Grid 512 (not 128 like most sibling toggle tests): the scheduler this toggle drives is keyed
//! to the LOD block / coarse tile (`block_size == grid/64`), which only exists as a genuine
//! 8x8-cell tile at the production resolution -- at 128 a tile is 2x2 cells, which exercises the
//! mechanism far less representatively.

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape};
use glam::Vec2;

/// FNV-1a checksum over every buffer `update` can mutate -- same construction as every sibling
/// toggle test in this directory.
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

/// Grid 512, Hourglass, Water, overfill on (the coarse level's own dynamics -- and therefore
/// `|Delta|`, the scheduler's signal -- run whenever `coarse.available`, independent of
/// `coarse_pressure_coupling`, which this deliberately leaves at its own default `false` so this
/// test isolates the scheduler from the driving-potential coupling).
fn run(touch: impl FnOnce(&mut DrawingSimulation), ticks: usize) -> (u64, f64, f64) {
    let mut sim = DrawingSimulation::new_with_size(512);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();
    touch(&mut sim);

    let m0: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.budget_n = 1024;
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 0.0, 16.6);
    }
    let m1: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    (checksum(&sim), m0, m1)
}

#[test]
fn overclocking_left_untouched_matches_explicitly_disabled() {
    let (untouched, ..) = run(|_sim| {}, 300);
    let (explicitly_off, ..) = run(|sim| sim.overclocking_enabled = false, 300);
    assert_eq!(
        untouched, explicitly_off,
        "never touching overclocking_enabled must be indistinguishable from explicitly setting \
         it false -- the field's default (set in DrawingSimulation::new_with_size) must be \
         false, matching every debug toggle in this codebase except the one that predates this \
         convention"
    );
}

#[test]
fn overclocking_enabled_diverges_from_default() {
    let (default_off, ..) = run(|_sim| {}, 300);
    let (forced_on, ..) = run(|sim| sim.overclocking_enabled = true, 300);
    assert_ne!(
        default_off, forced_on,
        "overclocking_enabled=true produced byte-identical output to the default single-rate \
         path over 300 ticks of a draining Hourglass at grid 512 -- either the toggle isn't \
         reaching DrawingSimulation::update's scheduler, or this scenario develops no \
         coarse-fine disagreement worth acting on"
    );
}

/// The non-negotiable property (OVERCLOCKING.md): turning the scheduler on must not raise
/// `mass_err` into anything that would read as a real leak. `diag_overclock_ab`'s `cost` section
/// measured this in the 1e-9 range with the toggle on at grid 512/Hourglass/Water, several times
/// smaller than a bound that would actually catch a leak -- 1e-5 is chosen generously above every
/// measured value (including the ~2.5e-8 the *default OFF* path itself sometimes shows) so this
/// is a real leak detector, not a re-assertion of today's exact noise floor.
#[test]
fn overclocking_enabled_does_not_leak_mass() {
    let (_, m0, m1) = run(|sim| sim.overclocking_enabled = true, 300);
    let mass_err = (m1 - m0).abs() / m0.max(1e-12);
    assert!(
        mass_err < 1e-5,
        "overclocking_enabled=true produced mass_err={mass_err:.3e} over 300 ticks of a \
         draining Hourglass at grid 512 -- real mass only ever moves through the fine edge \
         solver (conservative by construction), so a leak here means the scheduler's extra \
         settle_tick repetitions or the underclock-skip mechanism are corrupting \
         last_displacements/last_simulated_ticks bookkeeping in a way that loses or fabricates \
         material, not just deferring or reordering when it moves"
    );
}
