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

/// RENAMED 2026-08-16. This was called `..._conduction_and_rise` and asserted nothing whatever
/// about rise -- only mass conservation and "the bottom conduit filled", both of which hold while
/// water runs downhill and stops dead at the foot of the riser, which is the actual open defect.
/// A green test named for a property it does not check is worse than no test; the rise property
/// now lives in `spec_task70_u_tube_water_rises_up_the_riser` (parked, with the diagnosis).
#[test]
fn overfill_pressure_u_tube_flow_through_fills_the_basin() {
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

// ---------------------------------------------------------------------------------------------
// U-tube flow-through instrument (#70).
//
// The regions below are DERIVED from `physics::U_TUBE_RECTS`, evaluated at w = h = 128 with
// `center = 64`, `cx = dx + 64`, `cy = dy + 64`. Writing them out rather than importing the
// const because `U_TUBE_RECTS` is `pub(crate)`; the derivation is recorded here so a geometry
// change that silently invalidates these probes is at least traceable:
//
//   rect 0  reservoir / left arm  x [-0.42, -0.24) y [-0.40, 0.36)  ->  cx 11..33, cy 13..110
//   rect 1  basin (bottom)        x [-0.42,  0.02) y [ 0.36, 0.42)  ->  cx 11..66, cy 111..117
//   rect 2  right arm (THE RISER) x [-0.04,  0.02) y [ 0.10, 0.36)  ->  cx 59..66, cy  77..110
//   rect 3  spout                 x [-0.04,  0.16) y [ 0.10, 0.17)  ->  cx 59..84, cy  77..85
//   rect 4  catch well            x [ 0.16,  0.42) y [ 0.10, 0.42)  ->  cx 84..117, cy 77..117
//
// `initialize_hourglass` prefills rect 0 ONLY (see `lib.rs`, `U_TUBE_RESERVOIR_RECT`). So the
// apparatus is a reservoir that must drain down its own arm, cross the basin, and then RISE
// ~33 rows up the right arm before a single unit of mass can reach the spout or the catch well.
//
// THE PREVIOUS VERSION OF THIS INSTRUMENT MEASURED NONE OF THAT. It skipped
// `initialize_hourglass()` entirely (so every region started full of default fill and "highest
// filled row" read 0 from tick 0, making rise unobservable), and its "Right (Rise)" probe at
// x 90..115 was the CATCH WELL, not the riser. Do not reintroduce either mistake: the riser is
// x 59..66, and the whole point of this vessel is that reaching the catch well REQUIRES upward
// transport, so catch-well mass is the end-to-end signal and riser fill height is the local one.
// Ranges are half-open and INCLUSIVE of the rect's last row/column -- the derivation above gives
// inclusive cell indices, so each `end` is that index + 1. Getting this wrong clips the riser's
// bottom row, which is precisely the row the first unit of risen water lands in, and reads as a
// false zero.
const RESERVOIR_X: std::ops::Range<usize> = 11..34;
const RESERVOIR_Y: std::ops::Range<usize> = 13..111;
const BASIN_X: std::ops::Range<usize> = 11..67;
const BASIN_Y: std::ops::Range<usize> = 111..118;
const RISER_X: std::ops::Range<usize> = 59..67;
const RISER_Y: std::ops::Range<usize> = 77..111;
const CATCH_X: std::ops::Range<usize> = 84..118;
const CATCH_Y: std::ops::Range<usize> = 77..118;

/// Total mass in a rectangular probe.
fn region_mass(sim: &DrawingSimulation, w: usize, xs: std::ops::Range<usize>, ys: std::ops::Range<usize>) -> f32 {
    ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
        .map(|(x, y)| sim.heightmap.data[y * w + x])
        .sum()
}

/// Topmost row in a probe holding material, as rows ABOVE the probe's floor -- so a bigger
/// number always means "stands higher", independent of the y-down grid convention. Returns 0
/// when the probe is empty.
fn fill_height(sim: &DrawingSimulation, w: usize, xs: std::ops::Range<usize>, ys: std::ops::Range<usize>) -> usize {
    let floor = ys.end;
    ys.clone()
        .find(|&y| xs.clone().any(|x| sim.heightmap.data[y * w + x] > 0.1))
        .map(|top| floor - top)
        .unwrap_or(0)
}

fn build_u_tube() -> DrawingSimulation {
    let mut sim = DrawingSimulation::new_with_size(128);
    sim.sandbox_shape = SandboxShape::UTubeFlowThrough;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.overfill_capacity = 1.90;
    sim.initialize_hourglass();
    sim
}

fn step_u_tube(sim: &mut DrawingSimulation, ticks: usize) {
    let targets = [None; 5];
    for _ in 0..ticks {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::UTubeFlowThrough, 16.0, 16.0);
    }
}

/// #70: water must RISE up the right arm of the flow-through U-tube.
///
/// This is the property `overfill_pressure_u_tube_flow_through_conduction_and_rise` is NAMED for
/// and does not test -- that one asserts only mass conservation plus "the bottom conduit filled",
/// both of which are satisfied by water running downhill and stopping, which is exactly the
/// user-reported defect (deployed build 642b380, 2026-08-16: reservoir drains, basin fills, riser
/// stays empty). Reaching the riser at all requires upward transport through a saturated basin,
/// so riser fill height is the direct measurement.
///
/// ROOT CAUSE, recorded because two plausible-looking wrong answers were measured first and both
/// would have shipped as "fixes" (2026-08-16):
///
/// `cell_freecap[i]` -- arbitration's per-cell acceptor budget -- carries a documented contract
/// that it be a pure function of cell `i` (see the frozen-Jacobi buffer comment in `physics.rs`).
/// The overfill code wrote the per-EDGE limits `max_accept_fwd`/`max_accept_bwd` into it instead,
/// and those depend on the far endpoint, including via `.min(h_donor)`. Two edges write each cell,
/// so the surviving value depended on sweep order -- and it went wrong in exactly one
/// configuration: a cell with EMPTY space above it. The downward edge from that empty cell
/// contributes `max_accept_fwd = 0`, because an empty donor has nothing to give. When that write
/// landed last, the cell's acceptor budget was 0 and arbitration scaled its perfectly valid
/// upward flux from below to exactly zero. Every tick, at every rising water front. Laterally the
/// same bad write is nearly always harmless, because a lateral neighbour usually HAS mass -- which
/// is precisely why the defect presented as "sideways works, upward does not".
///
/// The signature was unmistakable once REALISED heights were read instead of candidate fluxes:
/// the vertical edge proposed `cand = -1.000000` (a full cell, the solver's maximum) every single
/// tick, while the cell above held `0.0000` and the cell below sat at the `1.900` ceiling.
/// Proposal at maximum and realisation at zero locates the loss between candidate and apply --
/// i.e. arbitration -- and rules out the driving head and the acceptance rule.
///
/// TWO WRONG ANSWERS, so they are not re-derived:
/// 1. "The vertical pass is missing the convective through-flow term the lateral pass has." True
///    as a divergence and worth unifying, but acceptance was never the binding constraint -- the
///    candidate was already at the solver's maximum.
/// 2. "The ceiling asymptote reports ~1e7 driving head, rails the velocity clamp, and the transfer
///    oscillates at the Nyquist frequency." The 1e7 head and the ceiling packing were both real,
///    but they were CONSEQUENCES of blocked outflow, not the cause. With arbitration fixed the
///    riser foot settles around 0.65 -- below capacity -- and never approaches the ceiling.
///    Reading candidate fluxes as if they were realised transfers is what made this look like an
///    oscillation; candidates are proposals and arbitration scales them.
#[test]
fn spec_task70_u_tube_water_rises_up_the_riser() {
    let w = 128;
    let mut sim = build_u_tube();
    let initial_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();

    step_u_tube(&mut sim, 4000);

    let final_mass: f64 = sim.heightmap.data.iter().map(|&v| v as f64).sum();
    let mass_err = (final_mass - initial_mass).abs() / initial_mass;
    assert!(mass_err < 1e-4, "mass not conserved: mass_err={:.8}", mass_err);

    let basin = region_mass(&sim, w, BASIN_X, BASIN_Y);
    let riser_h = fill_height(&sim, w, RISER_X, RISER_Y);
    let riser_m = region_mass(&sim, w, RISER_X, RISER_Y);
    assert!(basin > 50.0, "basin never filled, so the riser was never fed: basin={basin:.1}");
    assert!(
        riser_h >= 8,
        "water did not RISE up the riser: riser fill height = {riser_h} rows (of 33), \
         riser mass = {riser_m:.1}, basin mass = {basin:.1}. The basin is full and pressurised, \
         so this is upward transport failing, not a feed problem."
    );
}

/// Ground truth for the probe ranges above: prints the actual mask column profile so a derived
/// rect can be checked against the mask the solver really sees, rather than against arithmetic.
/// Run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_task70_u_tube_mask_profile() {
    let w = 128;
    let mut sim = build_u_tube();
    step_u_tube(&mut sim, 3000);
    for &x in &[20usize, 45, 58, 59, 62, 66, 67, 70, 90] {
        let rows: Vec<usize> = (0..w).filter(|&y| sim.shape_mask[y * w + x] != sandart_sim::MASK_OUTSIDE).collect();
        let span = match (rows.first(), rows.last()) {
            (Some(&a), Some(&b)) => format!("{a}..={b} ({} rows)", rows.len()),
            _ => "EMPTY".to_string(),
        };
        let filled: Vec<usize> = rows.iter().copied().filter(|&y| sim.heightmap.data[y * w + x] > 0.1).collect();
        let top = filled.first().map(|&y| y as i32).unwrap_or(-1);
        println!("x={x:3}: inside rows {span:22} | topmost filled row = {top:4} | contiguous={}",
            rows.windows(2).all(|p| p[1] == p[0] + 1));
    }
}

/// REALISED state at the riser foot, tick by tick, from the heightmap itself -- i.e. after
/// arbitration, not the per-edge candidate fluxes, which are only proposals and are scaled down
/// when several edges compete for one cell's free capacity. Reporting candidates as though they
/// were transfers is a mistake that has already been made once on this ticket.
///
/// Prints a vertical slice through the riser column (`x = 62`) plus the lateral neighbour that
/// feeds it, so "is the oscillation vertical or lateral" is answerable by looking. Rows 111 and
/// below are basin; 110 and above are riser. Run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_task70_riser_foot_realised_profile() {
    let w = 128;
    let mut sim = build_u_tube();
    step_u_tube(&mut sim, 4000);
    println!("      |            riser column x=62            | lateral feed row 112");
    println!("tick  | y=108  y=109  y=110  y=111  y=112  y=113 | x=56   x=58   x=60");
    for t in 0..16 {
        step_u_tube(&mut sim, 1);
        let at = |x: usize, y: usize| sim.heightmap.data[y * w + x];
        println!(
            "{:5} | {:5.3}  {:5.3}  {:5.3}  {:5.3}  {:5.3}  {:5.3} | {:5.3}  {:5.3}  {:5.3}",
            4000 + t + 1,
            at(62, 108), at(62, 109), at(62, 110), at(62, 111), at(62, 112), at(62, 113),
            at(56, 112), at(58, 112), at(60, 112),
        );
    }
}

/// #70: the overfill heat-map's saturation-decile legend.
///
/// Guards the three properties the overlay's readability actually rests on: the legend appears at
/// all (it is gated on the overlay being on, and a gate that never opens is the failure mode this
/// codebase has shipped before), the boundaries are sorted (they are read left-to-right as a
/// scale), and the colouring is genuinely EQUALISED -- no single decile may swallow the frame,
/// which is the whole reason for preferring deciles over a fixed scale.
#[test]
fn spec_task70_saturation_decile_legend() {
    let w = 128;
    let targets = [None; 5];
    let mut sim = DrawingSimulation::new_with_size(w);
    sim.sandbox_shape = SandboxShape::Square;
    sim.gravity_dir = Vec2::new(0.0, 0.04);
    sim.apply_preset(MaterialMode::Water);
    sim.overfill_pressure = true;
    sim.overfill_capacity = 1.90;
    sim.initialize_hourglass();

    // Overlay off: the legend must stay empty, because computing it is the cost this gate exists
    // to avoid.
    for _ in 0..90 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
    }
    assert!(
        sim.saturation_deciles.is_empty(),
        "deciles computed while the overlay is off: {:?}", sim.saturation_deciles
    );

    sim.pressure_heatmap_overlay = true;
    for _ in 0..90 {
        sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
    }

    let d = sim.saturation_deciles.clone();
    assert_eq!(d.len(), 9, "expected 9 decile boundaries: {d:?}");
    assert!(
        d.windows(2).all(|p| p[1] >= p[0]),
        "decile boundaries must be non-decreasing: {d:?}"
    );
    assert!(d[0] > 0.0, "lowest decile should be over an OCCUPIED cell, got {d:?}");

    // Equalisation check, over occupied cells only. Air is mapped to 0 and excluded from the
    // deciles, so it is excluded here too -- otherwise "most of the screen is empty" would look
    // like a failure of equalisation rather than a fact about the scene.
    let texels = sim.pressure_field_texels();
    let occupied: Vec<u8> = texels.iter().copied().filter(|&t| t > 0).collect();
    assert!(!occupied.is_empty(), "no occupied cells in the overlay");
    let mut hist = [0usize; 10];
    for &t in &occupied {
        // Inverse of the bucket -> byte map in `pressure_field_texels`.
        let bucket = (((t as usize).saturating_sub(1)) * 9 + 127) / 254;
        hist[bucket.min(9)] += 1;
    }
    let worst = *hist.iter().max().unwrap();
    assert!(
        worst * 2 <= occupied.len(),
        "colouring is not equalised -- one decile holds {worst} of {} occupied cells: {hist:?}",
        occupied.len()
    );
}

/// DOES THE TENSION BRANCH EARN ITS PLACE? Sweeps `underfill_tension` against the two properties
/// it has to satisfy simultaneously, at the shipped overfill capacity of 1.90.
///
/// Zero is included as the control -- it reproduces the pre-tension model exactly, so any row that
/// fails to beat it is an argument against the parameter, not for tuning it.
///
/// Reports, per setting: settled-pool stillness (must approach 0), the settled fill (must approach
/// 1.0 rather than the 1.90 ceiling), the SPREAD between the bottom and top decile of saturation
/// (the bimodality this branch exists to remove -- 1.16 on the deployed build), and free-fall
/// distance over a fixed window (must NOT regress; tension is not allowed to hold water up).
///
/// Run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_task70_underfill_tension_sweep() {
    let w = 128;
    let targets = [None; 5];
    println!("tension | pool amp | pool fill | decile spread | free-fall rows");
    for &tension in &[0.0f32, 0.25, 1.0, 4.0, 16.0] {
        // --- settled pool: stillness, fill, and the bimodality spread ---
        let mut sim = DrawingSimulation::new_with_size(w);
        sim.sandbox_shape = SandboxShape::Square;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(MaterialMode::Water);
        sim.overfill_pressure = true;
        sim.overfill_capacity = 1.90;
        sim.underfill_tension = tension;
        sim.pressure_heatmap_overlay = true;
        sim.initialize_hourglass();
        for _ in 0..3000 {
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
        }
        let probes: Vec<(usize, usize)> = (100..106).map(|y| (64usize, y)).collect();
        let read = |sim: &DrawingSimulation| -> Vec<f32> {
            probes.iter().map(|&(x, y)| sim.heightmap.data[y * w + x]).collect()
        };
        let mut prev = read(&sim);
        let (mut amp, mut fill) = (0.0f32, 0.0f32);
        const WINDOW: usize = 60;
        for _ in 0..WINDOW {
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
            let now = read(&sim);
            amp += now.iter().zip(&prev).map(|(a, b)| (a - b).abs()).sum::<f32>();
            fill += now.iter().sum::<f32>();
            prev = now;
        }
        let n = (WINDOW * probes.len()) as f32;
        let d = sim.saturation_deciles.clone();
        let spread = if d.len() == 9 { d[8] - d[0] } else { f32::NAN };

        // --- free fall: a slab dropped into empty space, unchanged by tension ---
        let mut fall = DrawingSimulation::new_with_size(w);
        fall.sandbox_shape = SandboxShape::Square;
        fall.gravity_dir = Vec2::new(0.0, 0.04);
        fall.apply_preset(MaterialMode::Water);
        fall.overfill_pressure = true;
        fall.overfill_capacity = 1.90;
        fall.underfill_tension = tension;
        fall.heightmap = Heightmap::new(w, w, 0.0);
        for y in 20..26 {
            for x in 56..72 {
                fall.heightmap.data[y * w + x] = 1.0;
            }
        }
        for _ in 0..200 {
            fall.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
        }
        let leading = (0..w)
            .rev()
            .find(|&y| (56..72).any(|x| fall.heightmap.data[y * w + x] > 0.1))
            .unwrap_or(0);

        println!(
            "{tension:7.2} | {:8.4} | {:9.3} | {:13.3} | {:14}",
            amp / n, fill / n, spread, leading as i32 - 25
        );
    }
}

/// A RESTING POOL MUST BE STILL. The controlled version of the capacity sweep below.
///
/// The U-tube sweep is confounded: changing `overfill_capacity` changes the entire flow, so
/// fixed-coordinate probes sample different physical situations at each setting (three of its five
/// rows have a near-EMPTY vertical probe, whose zero amplitude means nothing). This uses a plain
/// square vessel filled with water and left to settle, where the correct answer is known a priori
/// and is the same at every capacity: a body of water at rest must not move. Any tick-to-tick
/// change is pure numerical artifact, so amplitude is directly comparable across settings, and a
/// vertical pair and a lateral pair are sampled from the SAME settled pool.
///
/// Run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_task70_settled_pool_stillness_vs_capacity() {
    let w = 128;
    let targets = [None; 5];
    println!("overfill_cap | vert amp | vert fill | lat amp | lat fill");
    for &cap in &[1.00f32, 1.10, 1.30, 1.50, 1.90] {
        let mut sim = DrawingSimulation::new_with_size(w);
        sim.sandbox_shape = SandboxShape::Square;
        sim.gravity_dir = Vec2::new(0.0, 0.04);
        sim.apply_preset(MaterialMode::Water);
        sim.overfill_pressure = true;
        sim.overfill_capacity = cap;
        sim.initialize_hourglass();
        for _ in 0..3000 {
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
        }

        // Probe pairs taken deep inside the settled body, away from the free surface and the
        // walls: a vertical run of rows in one column, and a lateral run of columns in one row.
        let vertical: Vec<(usize, usize)> = (100..106).map(|y| (64usize, y)).collect();
        let lateral: Vec<(usize, usize)> = (40..46).map(|x| (x, 103usize)).collect();
        let sample = |sim: &DrawingSimulation, p: &[(usize, usize)]| -> Vec<f32> {
            p.iter().map(|&(x, y)| sim.heightmap.data[y * w + x]).collect()
        };
        let (mut prev_v, mut prev_l) = (sample(&sim, &vertical), sample(&sim, &lateral));
        let (mut sum_v, mut sum_l, mut fill_v, mut fill_l) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        const WINDOW: usize = 60;
        for _ in 0..WINDOW {
            sim.update(0.016, &targets, 0.08, MaterialMode::Water, SandboxShape::Square, 16.0, 16.0);
            let (nv, nl) = (sample(&sim, &vertical), sample(&sim, &lateral));
            sum_v += nv.iter().zip(&prev_v).map(|(a, b)| (a - b).abs()).sum::<f32>();
            sum_l += nl.iter().zip(&prev_l).map(|(a, b)| (a - b).abs()).sum::<f32>();
            fill_v += nv.iter().sum::<f32>();
            fill_l += nl.iter().sum::<f32>();
            prev_v = nv;
            prev_l = nl;
        }
        let n_v = (WINDOW * vertical.len()) as f32;
        let n_l = (WINDOW * lateral.len()) as f32;
        println!(
            "{cap:12.2} | {:8.4} | {:9.3} | {:7.4} | {:8.3}",
            sum_v / n_v, fill_v / n_v, sum_l / n_l, fill_l / n_l
        );
    }
}

/// WHY IS THE OSCILLATION VERTICAL-ONLY? Discriminator between the two candidate causes.
///
/// HYPOTHESIS A (overfill removes the brake): gravity puts a permanent driving head of
/// `base_head` on EVERY vertical edge, including deep inside a settled column. What used to stop
/// that column moving was `edge_sleeps`' first branch -- the acceptor is at capacity, so it has no
/// room and the edge is skipped. Overfill raises the effective capacity to `1.0 + overfill_ratio`,
/// so a cell at nominal capacity still reports `cap_eff - h` of room and the brake never engages;
/// gravity then pumps each row toward the ceiling indefinitely. Laterally two level cells have a
/// driving head of exactly ZERO, so `edge_sleeps`' second branch fires whatever the capacity is.
/// That asymmetry is the whole prediction: amplitude should scale with `overfill_capacity` and
/// collapse as it approaches 1.0.
///
/// HYPOTHESIS B (sweep-order parity): the solver flips sweep parity every tick, and a period-2
/// signal is exactly what that produces. This would be indifferent to `overfill_capacity`.
///
/// The two are separated by one sweep of the slider. Amplitude here is the mean absolute
/// tick-to-tick change in cell height over the sampling window -- a settled cell reads ~0.
/// Run with `-- --ignored --nocapture`.
#[test]
#[ignore]
fn diag_task70_oscillation_vs_overfill_capacity() {
    let w = 128;
    // A vertical-interior probe inside the riser column, and a lateral-interior probe inside the
    // basin, so "vertical oscillates, lateral does not" is read off the same run.
    let vertical = [(62usize, 108usize), (62, 110), (62, 112)];
    let lateral = [(30usize, 114usize), (34, 114), (38, 114)];
    // Mean height at each probe is printed alongside, because an EMPTY probe also reads zero
    // amplitude -- without it, "no oscillation at capacity 1.0" is indistinguishable from "no
    // water reached the probe at capacity 1.0", and the second says nothing.
    println!("overfill_cap | vert amp | vert fill | lat amp | lat fill | vert:lat");
    for &cap in &[1.00f32, 1.10, 1.30, 1.50, 1.90] {
        let mut sim = build_u_tube();
        sim.overfill_capacity = cap;
        step_u_tube(&mut sim, 4000);
        let (mut fill_v, mut fill_l) = (0.0f32, 0.0f32);

        let sample = |sim: &DrawingSimulation, probes: &[(usize, usize)]| -> Vec<f32> {
            probes.iter().map(|&(x, y)| sim.heightmap.data[y * w + x]).collect()
        };
        let mut prev_v = sample(&sim, &vertical);
        let mut prev_l = sample(&sim, &lateral);
        let (mut sum_v, mut sum_l) = (0.0f32, 0.0f32);
        const WINDOW: usize = 60;
        for _ in 0..WINDOW {
            step_u_tube(&mut sim, 1);
            let now_v = sample(&sim, &vertical);
            let now_l = sample(&sim, &lateral);
            sum_v += now_v.iter().zip(&prev_v).map(|(a, b)| (a - b).abs()).sum::<f32>();
            sum_l += now_l.iter().zip(&prev_l).map(|(a, b)| (a - b).abs()).sum::<f32>();
            fill_v += now_v.iter().sum::<f32>();
            fill_l += now_l.iter().sum::<f32>();
            prev_v = now_v;
            prev_l = now_l;
        }
        let amp_v = sum_v / (WINDOW * vertical.len()) as f32;
        let amp_l = sum_l / (WINDOW * lateral.len()) as f32;
        let fill_v = fill_v / (WINDOW * vertical.len()) as f32;
        let fill_l = fill_l / (WINDOW * lateral.len()) as f32;
        let ratio = if amp_l > 1e-6 { format!("{:.1}x", amp_v / amp_l) } else { "n/a".to_string() };
        println!("{cap:12.2} | {amp_v:8.4} | {fill_v:9.3} | {amp_l:7.4} | {fill_l:8.3} | {ratio:>9}");
    }
}

/// Time series behind `spec_task70_u_tube_water_rises_up_the_riser`. Run with
/// `-- --ignored --nocapture`. Prints the reservoir draining, the basin filling and -- the number
/// under investigation -- the riser's fill height in rows, plus catch-well mass as the end-to-end
/// "water made it over the spout" signal.
#[test]
#[ignore]
fn diag_task70_u_tube_rise_time_series() {
    let w = 128;
    let mut sim = build_u_tube();
    // `foot_m` is the basin mass in the columns DIRECTLY BENEATH the riser, over the 7 cells that
    // actually own the upward edges into it. Total basin mass can look healthy while the riser's
    // own foot is starved, and those are different defects.
    println!("tick | reservoir_h  basin_m  foot_m/cell  riser_h  riser_m  catch_m");
    for checkpoint in 0..=20 {
        if checkpoint > 0 {
            step_u_tube(&mut sim, 1000);
        }
        let foot_cells = (RISER_X.len() * BASIN_Y.len()) as f32;
        println!(
            "{:4} | {:10}  {:7.1}  {:11.3}  {:7}  {:7.1}  {:7.1}",
            checkpoint * 1000,
            fill_height(&sim, w, RESERVOIR_X, RESERVOIR_Y),
            region_mass(&sim, w, BASIN_X, BASIN_Y),
            region_mass(&sim, w, RISER_X, BASIN_Y) / foot_cells,
            fill_height(&sim, w, RISER_X, RISER_Y),
            region_mass(&sim, w, RISER_X, RISER_Y),
            region_mass(&sim, w, CATCH_X, CATCH_Y),
        );
    }
}
