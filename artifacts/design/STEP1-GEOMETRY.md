# Build step 1 — coarse geometry only

Status: **built and tested. Nothing reads it.** Executes HIERARCHICAL-PRESSURE.md §9 step 1:
"Coarse geometry only. Build `open_cells`, `capacity`, `k[e]` from the mask; add the connectivity
test against a fine flood fill. Nothing reads it yet."

## What was built

`sandart-sim/src/coarse.rs`, a new public module, `CoarseGeometry`:

- `open_cells[C]`, `capacity[C]`, `inside[C]` per coarse cell; `k_x`/`k_y` (conveyance fractions)
  per coarse edge, exactly as §4 defines `open_span[e]`/`k[e] = open_span[e]/t`.
- `CoarseGeometry::build(shape_mask, cell_props, grid_size)` — the production entry point, always
  pinned to `COARSE_GRID = 64` (§2's fixed-grid decision).
- `CoarseGeometry::refresh_capacity(shape_mask, cell_props)` — recomputes `capacity` only, leaving
  geometry (`open_cells`, `k_x`, `k_y`) untouched, as §4 asks for.
- `DrawingSimulation` gained one field, `pub coarse: CoarseGeometry`, rebuilt at the end of
  `generate_shape_mask()` and nowhere else — that function is the single call site every existing
  shape/neck-width/curve/flip/reset/grid-size-change setter already goes through, so no other
  wiring was needed or added. Nothing reads `self.coarse`.

**Grid-64 degenerate case (§2):** chose **disable**, not floor. `CoarseGeometry.available` is
`false` whenever `t < 2` (grid 64 and, hypothetically, below). Flooring `t` at 2 for grid 64 would
mean a 32x32 coarse grid there, which directly contradicts §2's own "the pressure grid is 64x64,
fixed, at every render resolution" — the fixed-64 decision exists specifically so `R` stays
resolution-invariant, and changing the coarse grid's shape below one resolution reopens exactly
that question. All vectors are still correctly sized and zero-filled when unavailable, so a future
caller that forgets to check `available` reads inert data rather than panicking.

**Capacity refresh cadence (§4's flagged exception):** not wired to anything yet (nothing reads
`capacity`), but documented in `refresh_capacity`'s doc comment: recommend the same cadence as
`SATURATION_DECILE_REFRESH_TICKS` (30 ticks) — not for the design's stated legibility reason
(that's a display concern), but because wetness mixes by advection, itself a diffusive process
slower than one tick, so a 30-tick-stale `capacity` is a small, bounded error against a
slow-moving quantity.

**`block_size` note (mid-task scope update from the coordinator):** the design's §2 anticipates
`block_size` becoming `grid_size/64` in a later, separate change, at which point the LOD block and
this coarse tile become the same footprint. This module was built with zero reference to
`block_size` — `COARSE_GRID = 64` is an independent constant, chosen specifically to equal that
future value so `t` and the future block edge length will coincide without any change to the
tiling math here. Only the wiring — and likely de-duplicating the `grid/64` arithmetic between the
two subsystems — will need revisiting once `block_size` actually changes. No `block_size` or
32x32-tiling code was touched in this step.

## Test results

`cargo test -p sandart-sim --lib --release`:

| | before (baseline, stashed) | after |
|---|---|---|
| passed | 91 | 98 |
| failed | 10 | 10 |
| ignored | 45 | 46 |

The 10 failures are byte-identical by name before and after (`test_dry_sand_has_angle_of_repose`,
`test_water_blob_stays_left_right_symmetric_under_gravity`, and the other 8 documented failures) —
zero new failures, zero fixed, none touched. +7 passing tests and +1 ignored diagnostic account
for the full delta. All six integration suites (`fresh_pressure_field_toggle`,
`head_field_transport_toggle`, `overfill_pressure_toggle`, `perfect_simulation_determinism`,
`pressure_heatmap_head_field_toggle`, `pressure_sensitive_flow_toggle`) pass unchanged, confirming
no behavioural change — `perfect_simulation_determinism` in particular is the strongest available
check that the sim is bit-for-bit identical.

New tests, in `coarse::tests`:

1. `connectivity_matches_fine_flood_fill_across_all_shapes_and_grids` — asserts `k[e] > 0` iff an
   independent 4-connected flood fill of the fine mask finds the boundary-straddling pair
   connected, across all 10 shipped `SandboxShape`s at grids 128/256/512 (30 combinations). PASS.
2. `diagonal_corner_touch_gives_zero_conveyance_on_both_shared_edges` — hand-built 4x4 mask,
   `coarse_n = 2`, two inside cells touching only at a diagonal corner. **Confirms the design's
   prediction**: `k = 0` on both shared edges, and (checked explicitly, not assumed) the two cells
   are not even in the same flood-fill component under 4-connectivity — the sharpest version of
   the corner case, and `k = 0` is correct.
3. `hourglass_neck_conveyance_at_512_is_a_sensible_fraction` — measured `k_y = 0.375` at the
   tightest boundary near the hourglass neck (grid 512, t=8). Sensible fraction, neither 0 nor 1.
   **Differs from §4's own illustrative `5/8 = 0.625`** — see "what the design got wrong" below.
4. `multistage_hourglass_floored_neck_conveyance_at_128` — measured global-minimum nonzero
   `k_y = 0.5` at grid 128 (t=2), exactly `1/2` — a floored 1-cell neck landing on a tile boundary.
   203 fractional (0<k<1) y-edges exist in this shape at this resolution (funnel walls taper
   continuously, producing many partial edges beyond just the neck).
5. `grid_64_is_unavailable_not_degenerate` — grid 64 == `COARSE_GRID` comes back `available =
   false`, correctly sized but inert.
6. `refresh_capacity_does_not_touch_geometry` — bumping all wetness to 1.0 and calling
   `refresh_capacity` alone changes `capacity` but leaves `open_cells`/`k_x`/`k_y` bit-identical.
7. `coarse_geometry_rebuilds_when_shape_mask_regenerates` — confirms the wiring: changing
   `sandbox_shape` and calling `generate_shape_mask()` changes `sim.coarse`.

## The neck-inside-a-tile measurement (§9 step 1's requested probe)

`#[ignore]`d diagnostic `coarse::tests::diag_neck_inside_tile_fraction` — run with
`cargo test -p sandart-sim --lib --release coarse::tests::diag_neck_inside_tile_fraction -- --ignored --nocapture`.

Method: per-row open-cell count over the fine mask; a row is a candidate "neck" if it is a local
minimum against both neighbours (strictly less than at least one, no worse than either), filtered
to nonzero rows. For each detected neck row `y`, "inside a tile" means `y % t` is neither `0` nor
`t-1` (i.e. not adjacent to any coarse boundary — no coarse edge can represent it at all).

| grid | shape | t | neck rows found | inside-tile | fraction | design's `(t-1)/t` |
|---|---|---|---|---|---|---|
| 128 | Hourglass | 2 | 27 | 0 | **0.000** | 0.500 |
| 128 | MultiStageHourglass | 2 | 22 | 0 | **0.000** | 0.500 |
| 256 | Hourglass | 4 | 55 | 29 | **0.527** | 0.750 |
| 256 | MultiStageHourglass | 4 | 43 | 20 | **0.465** | 0.750 |
| 512 | Hourglass | 8 | 107 | 78 | **0.729** | 0.875 |
| 512 | MultiStageHourglass | 8 | 84 | 64 | **0.762** | 0.875 |

**Grid 128 (t=2) is a measurement artifact, not a finding**: at `t=2` every row position is
`0` or `t-1 = 1` — there is no third position to be "inside", so this classifier structurally
cannot produce anything but 0.000 there. Ignore that row; it does not mean t=2 is safe.

At t=4 and t=8, the fraction is real and substantial but runs **below** the design's `(t-1)/t`
straight-line prediction (53%/47% measured vs 75% predicted at t=4; 73%/76% vs 87.5% at t=8) — see
below for the likely reason. Directionally the design's core worry is confirmed: **roughly half to
three-quarters of detected necks at realistic grids fall where no single coarse edge can represent
them**, and the fraction grows with `t`, i.e. gets worse exactly where the hierarchy is most
valuable (coarser tiles = bigger speedup, per §2's table).

## Where §4 turned out to be under-specified or measured-wrong

- **The `5/8 = 0.625` hourglass number in §4 is illustrative, not a measurement, and the real
  number is `0.375`.** §4 states "at 512 with `t=8` and a 5-cell neck, `k = 5/8 = 0.625`" as if it
  followed directly from the neck's total width. It doesn't: the neck's true minimum (5 cells) is
  a property of a single row at `dy=0` exactly, but the coarse boundary is between two *specific*
  discrete rows (255 and 256 at grid 512), which straddle `dy=0` asymmetrically because
  `allowed_hw` widens steeply away from center (`t.powf(hourglass_curve)` in
  `eval_sandbox_shape`). Row 255 is already wide open (all 8 columns of the tile inside); row 256
  is the tight one (3 of 8 inside). The boundary pair's intersection is 3, not 5, so `k = 3/8 =
  0.375`, not `5/8`. The 5-cell figure describes the neck's narrowest row in isolation; `k[e]`
  describes a specific *pair* of rows, and those are not the same measurement unless the tile
  boundary happens to land exactly on the narrowest row (it doesn't, generically, for any curved
  taper). This matters for §5/§7's later coupling: whoever tunes against "the neck should carry
  about `k=0.625`" will be tuning against a number that was never actually measured.

- **The `(t-1)/t` neck-inside-tile estimate over-predicts at every non-degenerate `t` measured**
  (see table above) — real fractions come in roughly 10-25 percentage points lower. Plausible
  reason: `(t-1)/t` assumes a neck's row is uniformly distributed relative to the tile grid, but a
  detected "local minimum" isn't a single row — curved tapers (the `t.powf(hourglass_curve)` term)
  produce short *runs* of consecutive rows near the true minimum with equal or near-equal width,
  and the local-minima detector as implemented only flags strict boundary transitions, which
  biases the sample. This is a second-order correction to the design's own headline number, not a
  refutation of its direction — the phenomenon is real and gets worse with `t`, just not linear
  in the simple way §4 states it.

- **`k[e]` for a diagonal corner touch is well-defined and correctly zero, and the design's own
  hedge ("this must be verified, not assumed") was warranted as a process point, not because the
  claim was wrong** — the verification was straightforward once written, and turned up nothing
  surprising. Worth noting for whoever reads this next: the "connectivity guarantee" test (test 1
  above) is close to tautological as literally stated in §4 (`k[e]>0` iff *some* fine pair on that
  boundary is inside — the count IS the definition), so its real value is as a regression check
  on the boundary-indexing arithmetic (off-by-one on `cx*t + (t-1)` etc.), not as a proof of a
  deep property. The flood-fill machinery was still worth building because it independently
  confirms the diagonal case is not merely "zero pairs on the boundary" but "genuinely
  disconnected under 4-connectivity" — a stronger and more useful fact than the tautology alone
  would give.

- **§4 doesn't say what "inside" means for `capacity`/`open_cells` precisely** (`MASK_BOUNDARY`
  vs `MASK_INSIDE`) — resolved here as `shape_mask != MASK_OUTSIDE` (i.e. `MASK_INSIDE` OR
  `MASK_BOUNDARY` both count), matching §4's own stated formula literally ("shape_mask !=
  MASK_OUTSIDE"), but this is worth flagging since `MASK_BOUNDARY` cells are adjacent to walls and
  a future capacity/pressure coupling might want to treat them differently (e.g. if boundary cells
  have systematically different wetness or packing behaviour near walls).

## Files

- `sandart-sim/src/coarse.rs` — new module, `CoarseGeometry` + tests.
- `sandart-sim/src/lib.rs` — `pub mod coarse;` + `pub use coarse::{CoarseGeometry, COARSE_GRID};`,
  new `DrawingSimulation.coarse` field, rebuild wired into `generate_shape_mask()`.
