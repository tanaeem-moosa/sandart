# #30 — 2.7 — Decile line still pinned: row_mass counts out-of-mask phantom cells

**Status:** completed

---

REOPENED 2026-07-31. The first fix (cache staleness, `QUANTILE_FULL_RESYNC_TICKS = 100`, shipped f652ec3) was a REAL bug and is correctly fixed, but it was not this one. The user still sees a pinned decile line at every resolution including 64.

## Actual root cause — confirmed by measurement
`refresh_row_mass_full` / `refresh_row_mass_active` (quantiles.rs) sum **raw `heightmap.data` with NO shape-mask filtering**; the call sites in lib.rs pass `&self.heightmap.data` unmasked. Cells outside the mask are never touched by the solver (every flux/CA path is gated by `is_inside`), so whatever height they hold is frozen forever — and the quantile cache counts it as real, moving sand.

Two places leave that phantom mass in place for Circle/Square/Oval:
- `reset()`'s non-hourglass branch calls `generate_smooth_noise`, which fills the ENTIRE w x h grid, not just the interior. The Hourglass branch (`initialize_hourglass`) zeroes out-of-mask cells afterwards; this one does not.
- `set_sandbox_shape` (sandart-wasm) for Circle/Square/Oval only calls `generate_shape_mask()` — it never zeroes newly-excluded cells. Contrast `flip_hourglass`, which does exactly this cleanup with a comment about this failure mode.

A circle inscribed in its bounding square leaves `1 - pi/4 ~= 21.5%` of the area outside the mask, concentrated in the extreme top/bottom rows where a row is only a couple of cells wide. Measured phantom fraction at 512: **Circle 0.335, Square 0.154, Oval 0.495**.

That alone clears the first decile's 10% threshold before the scan reaches real sand, so decile-1 freezes while deciles 2-9 descend normally. Measured, Circle DrySand 512: decile-1 row **69.96 at t=50, 500 AND 2000** (frozen), while decile-2 goes 138.83 -> 327.68 -> 334.12. Identical for Water — it is a caching bug, not material-specific. Never resolves at any runtime, because out-of-mask cells structurally cannot be touched.

Invisible to the eye because the renderer draws `MASK_OUTSIDE` as opaque casing. The user's "thin edge, not enough to be 10 percent" was accurate — they were judging something that is never drawn.

Proof of fix: re-computing row_mass filtered to `mask[idx] != MASK_OUTSIDE` makes decile-1 track properly — Circle 512 masked: 144.67 -> 356.84 -> 358.97, descending in step with the others.

## CORRECTION: there is no "diagonal ban"
An earlier hypothesis (mine) held that `ndy != 0.0 -> continue` banned diagonal flow and stranded sand against curved walls. That is WRONG on two counts, and it was propagated into this task and #34:
- `neighbors_info` is strictly 4-connected. Diagonal movement has NEVER existed in this simulator.
- The `ndy != 0.0` skip stops the CA from double-transferring mass across the same VERTICAL edge that Stage B moved onto the flux solver. It is a double-counting guard between two solvers sharing one edge, not a geometric restriction.
A mask-connectivity scan at 64/128/256/512 found ZERO interior cells with a blocked down-neighbour and no lateral escape, for Circle, Square and Oval. There is no geometric trap.

## The fix
Read-side only: add a mask parameter to `refresh_row_mass_full`/`refresh_row_mass_active` and sum only cells where `mask[idx] != MASK_OUTSIDE`, or pre-filter at the lib.rs call sites. Touches nothing in `physics.rs` — not `flux_edge`, not `edge_sleeps`, not the CA — so sand's simulated behaviour and appearance are provably unaffected.

## Separate decision, do NOT bundle
Zeroing out-of-mask cells at `reset()`/shape-switch time (mirroring `flip_hourglass`) is a second, independent fix. It touches `heightmap.data` writes rather than the quantile read path. Worth considering because phantom mass may distort OTHER measurements — anything summing total mass is currently including cells the solver cannot reach, which could mask a real leak. Raise it; do not bundle it.
