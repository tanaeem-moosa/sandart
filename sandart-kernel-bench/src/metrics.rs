//! Step 3 equivalence metrics, run purely natively (see `lib.rs`'s module doc comment for why
//! wasm parity is not re-checked here): total mass change, max/mean `|dh|`, incompressibility
//! (`max(h - capacity)`), max/mean `|d prop|` for wetness, mean `|d colour|`, and mirror symmetry
//! (`sum |h(x) - h(w-1-x)|` over inside cells).

use crate::consts::{MASK_OUTSIDE, PROP_WETNESS};
use crate::scalar_math::cell_capacity_for;
use crate::snapshot::State;

pub struct EquivReport {
    pub mass_a: f64,
    pub mass_b: f64,
    pub mass_delta: f64,
    pub max_abs_dh: f32,
    pub mean_abs_dh: f32,
    pub max_over_capacity: f32,
    pub max_over_capacity_ref: f32,
    pub max_over_capacity_test: f32,
    pub max_abs_dwetness: f32,
    pub mean_abs_dwetness: f32,
    pub mean_abs_dcolor: f32,
    pub mirror_asymmetry_a: f64,
    pub mirror_asymmetry_b: f64,
}

fn total_mass(s: &State) -> f64 {
    let mut m = 0.0f64;
    for i in 0..s.w * s.h {
        if s.shape_mask[i] != MASK_OUTSIDE {
            m += s.heights[i] as f64;
        }
    }
    m
}

fn mirror_asymmetry(s: &State) -> f64 {
    let w = s.w;
    let mut total = 0.0f64;
    for y in 0..s.h {
        for x in 0..w {
            let mx = w - 1 - x;
            let i = y * w + x;
            let mi = y * w + mx;
            if s.shape_mask[i] != MASK_OUTSIDE && s.shape_mask[mi] != MASK_OUTSIDE {
                total += (s.heights[i] - s.heights[mi]).abs() as f64;
            }
        }
    }
    total
}

fn max_over_capacity(s: &State) -> f32 {
    let mut worst = f32::MIN;
    for i in 0..s.w * s.h {
        if s.shape_mask[i] != MASK_OUTSIDE {
            let cap = cell_capacity_for(s.cell_props[i * 4 + PROP_WETNESS]);
            worst = worst.max(s.heights[i] - cap);
        }
    }
    worst
}

/// Compares kernel output `test` (e.g. A or B after N passes) against reference `reference`
/// (R after the same N passes), both starting from the identical snapshot.
pub fn compare(reference: &State, test: &State) -> EquivReport {
    let n = reference.w * reference.h;
    let mut max_abs_dh = 0.0f32;
    let mut sum_abs_dh = 0.0f64;
    let mut max_abs_dw = 0.0f32;
    let mut sum_abs_dw = 0.0f64;
    let mut sum_abs_dc = 0.0f64;
    let mut n_inside = 0usize;
    let mut n_color = 0usize;
    for i in 0..n {
        if reference.shape_mask[i] == MASK_OUTSIDE {
            continue;
        }
        n_inside += 1;
        let dh = (reference.heights[i] - test.heights[i]).abs();
        max_abs_dh = max_abs_dh.max(dh);
        sum_abs_dh += dh as f64;
        let dw = (reference.cell_props[i * 4 + PROP_WETNESS] - test.cell_props[i * 4 + PROP_WETNESS]).abs();
        max_abs_dw = max_abs_dw.max(dw);
        sum_abs_dw += dw as f64;
        for ch in 0..4 {
            let dc = (reference.cell_colors[i * 4 + ch] as i32 - test.cell_colors[i * 4 + ch] as i32).unsigned_abs();
            sum_abs_dc += dc as f64;
            n_color += 1;
        }
    }
    let mass_a = total_mass(reference);
    let mass_b = total_mass(test);
    let moc_ref = max_over_capacity(reference);
    let moc_test = max_over_capacity(test);
    EquivReport {
        mass_a,
        mass_b,
        mass_delta: mass_b - mass_a,
        max_abs_dh,
        mean_abs_dh: (sum_abs_dh / n_inside.max(1) as f64) as f32,
        max_over_capacity: moc_test.max(moc_ref),
        max_over_capacity_ref: moc_ref,
        max_over_capacity_test: moc_test,
        max_abs_dwetness: max_abs_dw,
        mean_abs_dwetness: (sum_abs_dw / n_inside.max(1) as f64) as f32,
        mean_abs_dcolor: (sum_abs_dc / n_color.max(1) as f64) as f32,
        mirror_asymmetry_a: mirror_asymmetry(reference),
        mirror_asymmetry_b: mirror_asymmetry(test),
    }
}
