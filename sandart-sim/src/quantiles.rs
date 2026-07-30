//! Cumulative-mass "quantile line" overlay for Sand-fall mode.
//!
//! The user-facing feature is a set of horizontal reference lines that mark where fixed
//! fractions of the total sand/liquid *mass* currently sit (e.g. "25% of the mass is above
//! this line"). As material falls, the lines descend. This module computes those line
//! positions from the heightmap; it never mutates simulation state — it is a pure observer.
//!
//! Design notes (see also the doc comments on `DrawingSimulation::row_mass` and
//! `DrawingSimulation::quantile_mode`):
//!
//! - **Row granularity.** `block_size` (16, giving 32 block-rows over the 512 grid) is far too
//!   coarse for 9 decile lines — they would cluster and visibly snap between block boundaries.
//!   We instead keep a per-grid-row mass cache (512 entries).
//! - **Cheap incremental refresh.** Re-summing all 512 rows every tick would add cost to a path
//!   the simulation doesn't otherwise need. Instead we reuse `active_blocks`, which
//!   `settle_tick` already populates with "was this block simulated this tick" — a row only
//!   needs to be re-summed if some block in its block-row was touched.
//! - **Bounded staleness via a periodic full resync.** The incremental refresh above only
//!   samples `active_blocks` on ticks it actually runs on (every 5th), which is a single-tick
//!   snapshot, not an OR across the ticks in between — a row a block changed and then went
//!   INACTIVE on an unsampled tick is never revisited, and can hold a stale cached mass
//!   indefinitely. `DrawingSimulation::update` (`sandart-sim/src/lib.rs`) calls
//!   `refresh_row_mass_full` unconditionally every `QUANTILE_FULL_RESYNC_TICKS` ticks (100) to
//!   bound how long that staleness can persist, independent of `has_active`/`active_blocks`.
//! - **No incremental accumulation.** Touched rows are always recomputed from scratch (a full
//!   sum over that row's cells), never adjusted by a running delta, so no f32 drift can
//!   accumulate in the cache over a long-running simulation.
//! - **Sub-row interpolation.** A quantile target is expressed as a fractional row position
//!   (`row + within_row_fraction`) so the line can move smoothly within a row instead of
//!   snapping a whole grid row at a time.

/// Which lines (if any) to draw. Off by default; the UI opts in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QuantileMode {
    Off,
    Quartiles,
    Deciles,
}

impl Default for QuantileMode {
    fn default() -> Self {
        Self::Off
    }
}

impl QuantileMode {
    /// The cumulative mass fractions this mode draws a line at, in ascending order.
    pub fn fractions(self) -> &'static [f32] {
        match self {
            QuantileMode::Off => &[],
            QuantileMode::Quartiles => &QUARTILE_FRACTIONS,
            QuantileMode::Deciles => &DECILE_FRACTIONS,
        }
    }

    pub fn line_count(self) -> usize {
        self.fractions().len()
    }
}

pub const QUARTILE_FRACTIONS: [f32; 3] = [0.25, 0.5, 0.75];
pub const DECILE_FRACTIONS: [f32; 9] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
/// Upper bound on the number of simultaneous quantile lines (deciles is the largest mode).
/// Also the flattening width used to pack line positions into GPU uniform buffer slots.
pub const MAX_QUANTILE_LINES: usize = 9;

/// Recompute the full per-row mass cache from scratch. Cost is O(width * height); intended to
/// be called only on discontinuities (reset, hourglass flip, or the quantile feature being
/// switched on) rather than every tick.
pub fn refresh_row_mass_full(heights: &[f32], width: usize, height: usize, row_mass: &mut Vec<f32>) {
    if row_mass.len() != height {
        row_mass.resize(height, 0.0);
    }
    if width == 0 || height == 0 {
        return;
    }
    for (y, slot) in row_mass.iter_mut().enumerate() {
        let start = y * width;
        *slot = heights[start..start + width].iter().sum();
    }
}

/// Refresh only the rows belonging to block-rows that were simulated this tick (per
/// `active_blocks`, populated by `physics::settle_tick`), leaving every other row's cached sum
/// untouched. Rows that are refreshed are always recomputed from scratch (never adjusted
/// incrementally), so no f32 drift can accumulate in cells that stay active for a long time.
pub fn refresh_row_mass_active(
    heights: &[f32],
    width: usize,
    height: usize,
    block_size: usize,
    active_blocks: &[crate::BlockActivity],
    row_mass: &mut Vec<f32>,
) {
    if row_mass.len() != height {
        row_mass.resize(height, 0.0);
    }
    if width == 0 || height == 0 || block_size == 0 {
        return;
    }
    let cols = (width + block_size - 1) / block_size;
    let block_rows = (height + block_size - 1) / block_size;

    for by in 0..block_rows {
        let row_touched = (0..cols).any(|bx| {
            active_blocks
                .get(by * cols + bx)
                .is_some_and(|a| *a != crate::BlockActivity::Inactive)
        });
        if !row_touched {
            continue;
        }
        let y_start = by * block_size;
        let y_end = ((by + 1) * block_size).min(height);
        for y in y_start..y_end {
            let start = y * width;
            row_mass[y] = heights[start..start + width].iter().sum();
        }
    }
}

/// Compute the sub-row-interpolated crossing position (normalised so 0.0 = the top edge of row
/// 0 and 1.0 = the bottom edge of the last row) for each requested cumulative mass fraction.
///
/// `fractions` must be sorted ascending (both `QUARTILE_FRACTIONS` and `DECILE_FRACTIONS` are);
/// this lets the whole set of quantiles be found in a single pass over `row_mass`.
///
/// Degenerate cases are handled explicitly rather than left to fall out of the arithmetic:
/// - **empty row_mass / no fractions requested**: returns an empty vec — nothing to draw.
/// - **zero total mass** (e.g. a freshly-created empty grid): returns `0.0` for every requested
///   fraction rather than dividing by zero.
/// - **mass concentrated in a single row**: every fraction's crossing lands inside that one row;
///   sub-row interpolation still spreads them out in fraction order (they remain monotonic).
/// - **fewer occupied rows than requested fractions** (e.g. 2 rows of sand but deciles want 9
///   lines): multiple fractions can legitimately land in the same row. That's expected, and the
///   result stays monotonically non-decreasing.
pub fn compute_quantile_positions(row_mass: &[f32], fractions: &[f32]) -> Vec<f32> {
    let height = row_mass.len();
    if height == 0 || fractions.is_empty() {
        return Vec::new();
    }

    let total_mass: f32 = row_mass.iter().sum();
    if total_mass <= f32::EPSILON {
        return vec![0.0; fractions.len()];
    }

    let mut positions = Vec::with_capacity(fractions.len());
    let mut row = 0usize;
    let mut cum_before = 0.0f32; // cumulative mass fraction strictly before `row`

    for &q in fractions {
        let q = q.clamp(0.0, 1.0);
        loop {
            let row_frac = row_mass[row] / total_mass;
            let cum_after = cum_before + row_frac;
            if cum_after >= q || row + 1 >= height {
                let within = if row_frac > f32::EPSILON {
                    ((q - cum_before) / row_frac).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                positions.push((row as f32 + within) / height as f32);
                break;
            }
            cum_before = cum_after;
            row += 1;
        }
    }

    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_column_median_at_centre() {
        let height = 512;
        let row_mass = vec![1.0f32; height];
        let positions = compute_quantile_positions(&row_mass, &[0.5]);
        assert_eq!(positions.len(), 1);
        assert!(
            (positions[0] - 0.5).abs() < 1e-3,
            "expected median at centre, got {}",
            positions[0]
        );
    }

    #[test]
    fn test_uniform_column_quartiles_ordered_and_centered() {
        let height = 512;
        let row_mass = vec![1.0f32; height];
        let positions = compute_quantile_positions(&row_mass, &QUARTILE_FRACTIONS);
        assert_eq!(positions.len(), 3);
        assert!((positions[0] - 0.25).abs() < 1e-2);
        assert!((positions[1] - 0.5).abs() < 1e-2);
        assert!((positions[2] - 0.75).abs() < 1e-2);
    }

    #[test]
    fn test_empty_grid_does_not_panic_or_divide_by_zero() {
        let row_mass = vec![0.0f32; 512];
        let positions = compute_quantile_positions(&row_mass, &DECILE_FRACTIONS);
        assert_eq!(positions.len(), DECILE_FRACTIONS.len());
        for p in positions {
            assert!(p.is_finite(), "position must not be NaN/Inf on empty grid");
            assert_eq!(p, 0.0);
        }

        // Truly empty row_mass slice (zero rows) must not panic either.
        let empty: Vec<f32> = Vec::new();
        let positions = compute_quantile_positions(&empty, &DECILE_FRACTIONS);
        assert!(positions.is_empty());
    }

    #[test]
    fn test_quantiles_monotonically_ordered_deciles() {
        // A non-trivial, non-uniform distribution: a ramp plus an empty gap plus a spike.
        let mut row_mass = vec![0.0f32; 512];
        for (y, slot) in row_mass.iter_mut().enumerate().take(200) {
            *slot = 1.0 + (y as f32) * 0.05;
        }
        row_mass[300] = 500.0; // spike
        for slot in row_mass.iter_mut().skip(400) {
            *slot = 2.0;
        }

        let positions = compute_quantile_positions(&row_mass, &DECILE_FRACTIONS);
        assert_eq!(positions.len(), 9);
        for pair in positions.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "quantile positions must be monotonically ordered: {:?}",
                positions
            );
        }
    }

    #[test]
    fn test_mass_concentrated_at_top_puts_all_quantiles_high() {
        // "Top" = low row index (row 0 is the upper chamber in initialize_hourglass).
        let mut row_mass = vec![0.0f32; 512];
        for slot in row_mass.iter_mut().take(50) {
            *slot = 1.0;
        }

        let positions = compute_quantile_positions(&row_mass, &DECILE_FRACTIONS);
        for &p in &positions {
            assert!(
                p < 50.0 / 512.0 + 1e-3,
                "expected quantile near the top (small normalised position), got {}",
                p
            );
        }
    }

    #[test]
    fn test_mass_concentrated_in_one_row() {
        let mut row_mass = vec![0.0f32; 512];
        row_mass[100] = 42.0;

        let positions = compute_quantile_positions(&row_mass, &QUARTILE_FRACTIONS);
        assert_eq!(positions.len(), 3);
        for &p in &positions {
            // All quantiles must land within row 100 (sub-row interpolated).
            assert!(p >= 100.0 / 512.0 && p <= 101.0 / 512.0);
        }
        // And still distinctly ordered within the row (sub-row interpolation, not integer snap).
        assert!(positions[0] < positions[1]);
        assert!(positions[1] < positions[2]);
    }

    #[test]
    fn test_fewer_occupied_rows_than_requested_quantiles() {
        // Only 2 occupied rows, but deciles asks for 9 lines.
        let mut row_mass = vec![0.0f32; 512];
        row_mass[10] = 1.0;
        row_mass[11] = 1.0;

        let positions = compute_quantile_positions(&row_mass, &DECILE_FRACTIONS);
        assert_eq!(positions.len(), 9);
        for &p in &positions {
            assert!(p.is_finite());
            assert!(p >= 10.0 / 512.0 && p <= 12.0 / 512.0);
        }
        for pair in positions.windows(2) {
            assert!(pair[1] >= pair[0]);
        }
    }

    #[test]
    fn test_refresh_row_mass_full_matches_direct_sum() {
        let width = 8;
        let height = 8;
        let mut heights = vec![0.0f32; width * height];
        for (i, v) in heights.iter_mut().enumerate() {
            *v = i as f32 * 0.01;
        }
        let mut row_mass = Vec::new();
        refresh_row_mass_full(&heights, width, height, &mut row_mass);

        for y in 0..height {
            let expected: f32 = heights[y * width..(y + 1) * width].iter().sum();
            assert!((row_mass[y] - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn test_refresh_row_mass_active_only_touches_active_block_rows() {
        let width = 8;
        let height = 8;
        let block_size = 4; // 2x2 blocks
        let heights = vec![1.0f32; width * height];
        let mut row_mass = vec![0.0f32; height];

        // Seed a stale/wrong cache value everywhere.
        row_mass.iter_mut().for_each(|v| *v = -999.0);

        // Only the top-left block (rows 0..4, cols 0..4) is active.
        let cols = 2;
        let rows = 2;
        let mut active_blocks = vec![crate::BlockActivity::Inactive; cols * rows];
        active_blocks[0] = crate::BlockActivity::Fast; // block (bx=0, by=0)

        refresh_row_mass_active(&heights, width, height, block_size, &active_blocks, &mut row_mass);

        // Rows 0..4 (the active block-row) must be refreshed to the real sum.
        for y in 0..4 {
            assert!((row_mass[y] - width as f32).abs() < 1e-6, "row {} should be refreshed", y);
        }
        // Rows 4..8 (untouched block-row) must retain the stale placeholder.
        for y in 4..8 {
            assert_eq!(row_mass[y], -999.0, "row {} should be untouched", y);
        }

        // Sanity: heights weren't mutated by a read-only refresh.
        assert!(heights.iter().all(|&h| h == 1.0));
    }
}
