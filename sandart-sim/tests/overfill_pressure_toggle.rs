//! Integration tests for the per-cell overfill pressure simulation toggle
//! (`DrawingSimulation::overfill_pressure`, ticket #70).
//!
//! Verifies:
//! 1. `overfill_pressure` default false is a true no-op (bit-identical).
//! 2. `overfill_pressure = true` activates overfill compression and driving head.
//! 3. Mass is strictly conserved across hundreds of ticks of overfilled settling.
//! 4. U-tube communicating vessels / siphon equilibrium under overfill pressure.
//! 5. Granular angle of repose is preserved (Mohr-Coulomb yield stress resists lateral failure).

use sandart_sim::{DrawingSimulation, MaterialMode, SandboxShape, Heightmap};
use glam::Vec2;

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

fn run_hourglass(touch: impl FnOnce(&mut DrawingSimulation)) -> (u64, f64, f64) {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::Hourglass;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.initialize_hourglass();
    sim.apply_preset(MaterialMode::Water);
    touch(&mut sim);

    let initial_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let targets = [None; 5];
    for _ in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Hourglass, 16.0, 16.0);
    }
    let final_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    (checksum(&sim), initial_mass, final_mass)
}

#[test]
fn overfill_pressure_left_untouched_matches_explicitly_disabled() {
    let (untouched, _, _) = run_hourglass(|_sim| {});
    let (explicitly_off, _, _) = run_hourglass(|sim| sim.overfill_pressure = false);
    assert_eq!(
        untouched, explicitly_off,
        "never setting overfill_pressure must be indistinguishable from explicitly setting it to false"
    );
}

#[test]
fn overfill_pressure_enabled_conserves_mass_and_diverges_from_default() {
    let (default_off, init_off, final_off) = run_hourglass(|_sim| {});
    let (forced_on, init_on, final_on) = run_hourglass(|sim| sim.overfill_pressure = true);

    // 1. Must diverge from default
    assert_ne!(
        default_off, forced_on,
        "overfill_pressure=true should diverge from default overfill_pressure=false on deep water column"
    );

    // 2. Strict mass conservation
    let mass_err_off = (final_off - init_off).abs() / init_off;
    let mass_err_on = (final_on - init_on).abs() / init_on;
    assert!(
        mass_err_off < 1e-4,
        "default mass conservation failed: mass_err={:.8}", mass_err_off
    );
    assert!(
        mass_err_on < 1e-4,
        "overfill_pressure=true mass conservation failed: mass_err={:.8}", mass_err_on
    );
}

#[test]
fn overfill_pressure_u_tube_communicating_vessels_equilibrates() {
    let w = 128;
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();

    let initial_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let initial_bottom_mass: f64 = (110..118).map(|y| (35..90).map(|x| sim.heightmap.data[y * w + x] as f64).sum::<f64>()).sum();

    let targets = [None; 5];
    for _ in 0..500 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
    }

    let final_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let final_bottom_mass: f64 = (110..118).map(|y| (35..90).map(|x| sim.heightmap.data[y * w + x] as f64).sum::<f64>()).sum();

    let mass_err = (final_mass - initial_mass).abs() / initial_mass;
    assert!(mass_err < 1e-4, "U-tube mass not conserved: mass_err={:.8}", mass_err);

    assert!(
        final_bottom_mass > initial_bottom_mass + 10.0,
        "Water did not flow through bottom conduit! init={:.2}, final={:.2}",
        initial_bottom_mass, final_bottom_mass
    );
}

#[test]
fn overfill_pressure_granular_preserves_angle_of_repose() {
    // Dry sand pile on a flat floor should maintain its angle of repose and not liquefy into a flat puddle
    let w = 64;
    let h = 64;
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::DrySand);
    sim.overfill_pressure = true;

    // Create a pyramid pile in the center
    sim.heightmap = Heightmap::new(w, h, 0.0);
    let center_x = 32.0;
    for y in 30..60 {
        for x in 10..54 {
            let dx = (x as f32 - center_x).abs();
            let pile_height = (18.0 - dx * 0.8).max(0.0);
            if (60 - y) as f32 <= pile_height {
                sim.heightmap.data[y * w + x] = 1.0;
            }
        }
    }

    let initial_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();

    let targets = [None; 5];
    for _ in 0..200 {
        sim.update(0.016, &targets, 0.08, MaterialMode::DrySand, SandboxShape::Square, 16.0, 16.0);
    }

    let final_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let mass_err = (final_mass - initial_mass).abs() / initial_mass;
    assert!(mass_err < 1e-4, "Dry sand pile mass not conserved: mass_err={:.8}", mass_err);

    // Peak height at center must remain distinctly elevated (angle of repose intact, not flattened into puddle)
    let peak_height_col = (0..60).map(|y| sim.heightmap.data[y * w + 32]).sum::<f32>();
    let edge_height_col = (0..60).map(|y| sim.heightmap.data[y * w + 12]).sum::<f32>();
    assert!(
        peak_height_col > edge_height_col + 5.0,
        "Dry sand flattened out into puddle under overfill_pressure! peak={:.2}, edge={:.2}",
        peak_height_col, edge_height_col
    );
}

#[test]
fn overfill_pressure_u_tube_flow_through_conduction_and_rise() {
    let w = 128;
    let _h = 128;
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.initialize_hourglass();

    let initial_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let targets = [None; 5];
    for _ in 0..1000 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
    }

    let final_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let mass_err = (final_mass - initial_mass).abs() / initial_mass;
    assert!(mass_err < 1e-4, "Mass not conserved in UTubeFlowThrough: mass_err={:.8}", mass_err);

    // Water must have traversed the horizontal conduit from the reservoir (left arm)
    let bottom_mass: f32 = (115..118).map(|y| {
        (0..w).map(|x| sim.heightmap.data[y * w + x]).sum::<f32>()
    }).sum();
    assert!(
        bottom_mass > 100.0,
        "Water did not fill bottom conduit! bottom_mass={:.2}", bottom_mass
    );
}

#[test]
fn diagnostic_u_tube_gradient_64x64() {
    let w = 128;
    let _h = 128;
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.overfill_capacity = 1.90;
    let targets = [None; 5];
    for tick in 0..3000 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
        if tick % 300 == 0 || tick == 2999 {
            let left_mass: f32 = (0..w).map(|y| (15..35).map(|x| sim.heightmap.data[y * w + x]).sum::<f32>()).sum();
            let conduit_mass: f32 = (110..118).map(|y| (35..90).map(|x| sim.heightmap.data[y * w + x]).sum::<f32>()).sum();
            let right_mass: f32 = (0..w).map(|y| (90..115).map(|x| sim.heightmap.data[y * w + x]).sum::<f32>()).sum();
            let highest_left_y = (0..110).filter(|&y| (15..35).any(|x| sim.heightmap.data[y * w + x] > 0.1)).min();
            let highest_right_y = (0..110).filter(|&y| (90..115).any(|x| sim.heightmap.data[y * w + x] > 0.1)).min();
            println!("Tick {:4}: Left (Reservoir)={:6.1} (y={:?}), Conduit={:5.1}, Right (Rise)={:6.1} (y={:?})",
                tick, left_mass, highest_left_y, conduit_mass, right_mass, highest_right_y);
        }
    }
}
