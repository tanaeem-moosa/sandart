//! Is the flow limiter the CFL clamp, or the overfill stiffness?
use sandart_sim::physics::{cell_potential, overfill_equilibrium_transfer, overfill_ceiling_for,
                           GRAVITY_HEAD_SCALE};

fn main() {
    let g = 1.0f32; // base_head = gravity_dir.y * GRAVITY_HEAD_SCALE = 0.04 * 25
    println!("per-edge transfer d (cells/tick) for a DOWNWARD liquid edge, gravity head {g}\n");
    println!("{:<10} {:>8} {:>10} {:>12} {:>12} {:>12} {:>12}",
             "grid", "stiff", "unit", "empty->", "half->", "full->full", "vs clamp 1.0");
    for &w in &[512usize, 256, 128] {
        for &stiff in &[5.0f32, 2.0, 1.0] {
            let depth_scale = 512.0 / w as f32;
            let unit = (GRAVITY_HEAD_SCALE / depth_scale) * stiff;
            let ratio = (overfill_ceiling_for(stiff) - 1.0).max(0.0);
            let (cap, cap_eff) = (1.0f32, 1.0 * (1.0 + ratio));
            let t = |h_a: f32, h_b: f32| overfill_equilibrium_transfer(
                h_a, cap, h_b, cap, cap_eff, cap_eff, g, 0.0, 0.0, 1.0, 1.0, ratio, unit, 1.0);
            let d_empty = t(1.0, 0.0);
            let d_half = t(1.0, 0.5);
            let d_full = t(1.0, 1.0);
            println!("{:<10} {:>8.1} {:>10.1} {:>12.4} {:>12.4} {:>12.5} {:>11.1}x",
                     w, stiff, unit, d_empty, d_half, d_full, 1.0 / d_full.max(1e-9));
        }
    }
    println!("\nsame, but for a LATERAL edge (gravity head 0) between two saturated cells with a");
    println!("small height difference -- this is what levels a pool:");
    println!("{:<10} {:>8} {:>14} {:>16}", "grid", "stiff", "dh = 0.01", "dh = 0.10");
    for &w in &[512usize, 128] {
        for &stiff in &[5.0f32, 1.0] {
            let depth_scale = 512.0 / w as f32;
            let unit = (GRAVITY_HEAD_SCALE / depth_scale) * stiff;
            let ratio = (overfill_ceiling_for(stiff) - 1.0).max(0.0);
            let (cap, cap_eff) = (1.0f32, 1.0 * (1.0 + ratio));
            let t = |h_a: f32, h_b: f32| overfill_equilibrium_transfer(
                h_a, cap, h_b, cap, cap_eff, cap_eff, 0.0, 0.0, 0.0, 1.0, 1.0, ratio, unit, 1.0);
            println!("{:<10} {:>8.1} {:>14.5} {:>16.5}", w, stiff, t(1.01, 1.00), t(1.10, 1.00));
        }
    }
    let _ = cell_potential;
}
