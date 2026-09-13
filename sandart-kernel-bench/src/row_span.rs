//! Shared "contiguous active span" finder for kernels A and B. Both process the lateral pass as
//! whole-array stages over spans of a grid row that a simulated block owns, rather than R's
//! per-block, per-edge traversal -- see `lib.rs`'s module doc comment for why that structural
//! difference is deliberate and where it does (and doesn't) save work relative to R.
//!
//! **Edge ownership, and why a span's right edge is asymmetric.** In the real solver (and in
//! kernel R), edge `(x, x+1)` is owned by `block(x)` alone -- `block(x+1)`'s own simulated/inactive
//! status never gates it. So a run of horizontally-adjacent SIMULATED blocks in one block-row
//! forms one contiguous x-range whose every internal edge is active regardless of mask, and whose
//! very last edge (from the run's last owned column into the immediately following column) is
//! ALSO active even if that following column's block is not simulated -- only that one extra
//! "acceptor" cell's data (never a further edge starting there) is needed. That is exactly the
//! `+1` extension below.

use crate::snapshot::State;
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

/// One contiguous run: grid row `y`, owned columns `[x_start, x_owned_end)` (every block in this
/// range is simulated), plus one extra readable "acceptor-only" column at `x_owned_end` when
/// `has_extra` is true (i.e. `x_owned_end < w`). Edges to evaluate are exactly
/// `x_start..x_owned_end` (edge `e` joins cell `e` and `e+1`); `e = x_owned_end - 1`'s acceptor is
/// the extra column, never a further edge.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub y: usize,
    pub x_start: usize,
    pub x_owned_end: usize,
    pub has_extra: bool,
}

impl Span {
    #[inline]
    pub fn data_end(&self) -> usize {
        self.x_owned_end + if self.has_extra { 1 } else { 0 }
    }
}

/// Builds every span for the snapshot's fixed `sim_blocks` list. Called once per kernel-run
/// (the block list never changes across repeated passes in this benchmark -- see
/// `snapshot::State::sim_blocks`'s doc comment), not once per pass.
pub fn build_spans(state: &State) -> Vec<Span> {
    build_spans_raw(state.cols, state.rows, state.block_size, state.w, state.h, &state.sim_blocks)
}

/// Same logic as `build_spans`, taking the handful of primitive fields it needs directly instead
/// of a `snapshot::State` -- so `kernel_e`'s own `StateE` (an SoA layout with no `snapshot::State`
/// inside it) can build the identical span list without an adapter struct.
pub fn build_spans_raw(cols: usize, rows: usize, block_size: usize, w: usize, h: usize, sim_blocks: &[u32]) -> Vec<Span> {
    let mut block_active = vec![false; cols * rows];
    for &b in sim_blocks {
        block_active[b as usize] = true;
    }

    let mut spans = Vec::new();
    for by in 0..rows {
        // Merge consecutive simulated bx into runs.
        let mut bx = 0usize;
        while bx < cols {
            if !block_active[by * cols + bx] {
                bx += 1;
                continue;
            }
            let run_start_bx = bx;
            while bx < cols && block_active[by * cols + bx] {
                bx += 1;
            }
            let run_end_bx = bx; // exclusive
            let x_start = run_start_bx * block_size;
            let x_owned_end = (run_end_bx * block_size).min(w);
            let has_extra = x_owned_end < w;
            let start_y = by * block_size;
            let end_y = ((by + 1) * block_size).min(h);
            for y in start_y..end_y {
                spans.push(Span { y, x_start, x_owned_end, has_extra });
            }
        }
    }
    spans
}
