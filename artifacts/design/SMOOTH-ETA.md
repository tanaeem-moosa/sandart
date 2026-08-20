# Smooth eta — interpolating the coarse potential onto every fine cell

Status: **built, measured, tree green.** Cut short by a priority change mid-measurement (see
"What is unfinished" at the end) — the coupling this fixes is going behind a flag defaulting OFF
for an unrelated, more fundamental reason, so this work is parked, not abandoned or wrong.

Files touched: `sandart-sim/src/coarse.rs`, `sandart-sim/src/physics.rs`. Nothing in
`sandart-render`, `sandart-wasm`, or the web front end — those are a concurrent agent's, untouched.
`sandart-sim/src/lib.rs` shows as modified in `git status` but that diff (`coarse_eta_texels` /
`coarse_delta_texels`) is the concurrent agent's overlay work, not mine — confirmed by inspection.

## The defect

The user turned on the pressure heat-map and could see block boundaries. Diagnosis (both user's
and design's, HIERARCHICAL-PRESSURE.md §0.2): `eta` was injected as a **constant per coarse tile**.
`coarse_delta_eta` returned `eta[tile_a] - eta[tile_b]` only when two fine cells fell in
**different** tiles (a 1-in-`t` fine edges at grid 512), and `0.0` otherwise. So the entire coarse
gradient landed on tile-seam edges and the `t-1` interior edges got nothing — a sawtooth locked to
block boundaries, and also the reason the coupling was measured to move so little material (§8's
311,196 hourglass "no bang-bang transport" firings — all the driving concentrated on seam edges).

## The fix

`sandart-sim/src/physics.rs`: replaced the tile-constant lookup with `eta_fine_interp`, a bilinear
interpolation of the 64x64 `eta` field onto each fine cell's own position (fine cell `fx` maps to
coarse-index space via `fx_c = (fx + 0.5)/t - 0.5`, so a fine cell exactly on a coarse centre gets
that centre's value and a cell midway between two centres gets their average — the form the task
asked for). `coarse_delta_eta(idx_a, idx_b)` is now `eta_fine_interp(idx_a) - eta_fine_interp(idx_b)`
for every edge, not just seam edges. Boundary cells (outer half-tile) clamp/extrapolate flat from
the outermost coarse row/column rather than reading off-grid.

**Mask handling (the flagged hazard).** `CoarseState::update_head_and_disagreement`
(`coarse.rs`) now writes `eta[C] = f32::NAN` for `!inside[C]` tiles (solid wall / outside geometry)
instead of `0.0` — `0.0` is a plausible genuine reading (e.g. at a free surface), so it cannot
double as "invalid" without silently corrupting a real one. `eta_fine_interp` drops NaN corners out
of its 4-corner blend and renormalises weight over whichever remain; if all 4 are invalid it
returns NaN itself, and `coarse_delta_eta` turns any NaN operand into a `0.0` delta before it ever
reaches the solver. This is "clamp/extrapolate from in-mask neighbours only" and "weight by
`inside[C]`" implemented as the same operation — dropping and renormalising IS extrapolating from
what's left.

**Empty-but-inside tiles are deliberately NOT gated out.** A tile with `inside[C] = true` but zero
mass gets `eta[C] = -cy * base_head_coarse` (elevation only, same "free surface at its own
elevation" convention the fine solver already uses for a dry cell). This is interpolated like any
other tile's `eta`, on purpose: excluding it would delete the gradient exactly at a flow front's
leading edge (empty tile next to a compressed one — precisely where material should want to flow),
and would reintroduce a smaller-scale version of the same seam artifact this change removes.

**I4 (flux budget) and I5 (deadband) are both untouched in mechanism.** I4's per-tile budget
(`coarse_delta_eta_budgeted` / `coarse_tile_indices`) still attributes only to each fine cell's own
HOME tile (not the interpolated blend) and still exempts same-home-tile edges from budget
consumption — documented as deliberate: an edge whose two cells share a home tile cannot change
that tile's `A[C]` regardless of how much crosses it, so there is nothing for I4 to attribute
against. I5's per-edge saturation clamp (`flux_edge_candidate`'s `.clamp(-1.0, 1.0)`) is unchanged
and unconditional, so it still bounds every edge including the now-nonzero intra-tile ones.

## Mechanical verification (tree state: green)

- `cargo check -p sandart-sim --release`: clean.
- `cargo test -p sandart-sim --lib --release`: **102 passed, 10 failed** — the same ten named
  failures as baseline (`test_task55_dynamic_transport_spec_scoreboard`,
  `test_dry_sand_has_angle_of_repose`, `test_head_field_transport_repose_non_regression`,
  `test_liquid_pool_levels_flat_in_closed_box`, `test_liquid_stream_stays_coherent`,
  `test_sandbox_wave_decays_to_flat_pool`, `test_sandbox_wave_reach_is_budget_independent`,
  `test_sandbox_wave_reflects_off_boundary`, `test_sandbox_wave_stays_left_right_symmetric`,
  `test_water_blob_stays_left_right_symmetric_under_gravity`). No new failures.
- All seven integration suites pass, including `coarse_pressure_coupling_toggle` and
  `perfect_simulation_determinism`.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`: clean.
- Re-confirmed clean a second time immediately before handoff.

## Measurements

Methodology: a clean `git worktree` at `adbd546` (pre-change HEAD, untouched by either agent's
uncommitted work) gave the "before" numbers without disturbing the concurrent agent's uncommitted
`sandart-render`/`sandart-wasm`/web changes in this working tree. "After" is this tree.

### 1. Block-boundary banding — the user's actual complaint

New instrument: `sandart-sim/examples/diag_smooth_eta_banding.rs`. Deep resting pool (Square
sandbox, grid 512, 800 ticks), `o = (h-cap)/cap` sampled down a 5-column-wide band near centre,
rows well clear of the free surface and floor. Two metrics: seam-row mean minus interior-row mean
(`y % block_size == 0` vs not), and DFT magnitude at the block period (`1/8` cycles/row).

| | seam-interior (o) | DFT @ 1/8 |
|---|---|---|
| coupling OFF (either tree, control) | 0.000023 | 0.000526 |
| coupling ON, **before** | **0.003912** | **0.001993** |
| coupling ON, **after** | **0.000120** | **0.000665** |

Before: the ON seam-interior offset is 170x the OFF noise floor, and the DFT component is 3.8x it —
a real, measurable periodic artifact at the block period. After: both are within ~2x of the OFF
noise floor. **The banding is fixed, quantitatively, not just by argument.**

### 2. Coupling strength — NOT measured to completion

`diag_coarse_ab` (pool-levelling ticks-to-50%, U-tube riser rise, settled churn, ms/tick ON vs OFF)
was launched on both the before-worktree and this tree but was killed mid-run when the priority
change arrived (each run takes several minutes; the given baseline in the task brief — 2434/2698
ticks pool levelling, 3.10e-5/2.89e-5 hourglass drain, 0.000054/0.000025 churn, 39.4/23.6 ms/tick —
is presumably still current for "before" but **was not re-verified**, and "after" has no number at
all). **This is the biggest gap in this report — see "What is unfinished."**

### 3. Saturation count — measured, and the result contradicts the task's stated expectation

`diag_coarse 512 64 400 [utube]`, `bang_bang_count` / `coarse_budget_clamp_count`:

| | hourglass bang-bang | hourglass I4 clamps | U-tube bang-bang | U-tube I4 clamps |
|---|---|---|---|---|
| before | 311,196 | 48,724 | 34,782 | 27,291 |
| after | **2,198,332** | **5,708** | **147,331** | **11,146** |

The bang-bang ("no bang-bang transport", §8) count **rose 7.1x (hourglass) and 4.2x (U-tube)**,
not "fell sharply" as predicted. The I4 flux-budget clamp count fell sharply instead (-88% / -59%).

Read together, this is explicable and worth reporting precisely rather than either hiding or
panicking over: the bang-bang counter increments on any edge where `coarse_head != 0.0` AND the
solver's full-mass-limit branch is hit. Before, only ~1-in-8 edges (true tile seams) ever had
nonzero `coarse_head` at all, so the counter's population was small and concentrated. After, nearly
every edge has some nonzero interpolated `coarse_head`, so the population is ~8x larger — and
because many fine vertical edges in a compressed column interior sit close to their transfer limit
from gravity/overfill alone (§1's `R*tau=o_max` identity: deep columns operate near the saturated
regime structurally), a small additional nonzero head is often enough to tip an already-near-limit
edge over the threshold. The I4 numbers support this reading: I4 does much LESS work post-fix
(fewer inter-tile edges carry a large enough share of the gradient to need budget clamping, because
the gradient is now spread thin), consistent with "less concentration" — but that same thinning
means many more, individually smaller, intra-tile edges are now touching the saturation branch,
and intra-tile edges are explicitly outside I4's scope (see "The fix" above).

**This coupling directly explains the coordinator's separately-arrived-at finding**: transport is
clamped at `.clamp(-1.0, 1.0)`, one cell per step, at every resolution, and this measurement shows
that clamp is already binding on a very large and (post-fix) growing fraction of coarse-coupled
edges. A potential term — smoothed or not — can only redirect where that one cell of movement goes;
it cannot make more of it happen. That is the load-bearing reason the coupling is going behind a
flag pending sub-stepping, independent of whether `eta` is smooth or seam-locked.

### 4. Criterion 4 — held, both instruments

`diag_coarse 512 64 400 [utube]`: falling-dominated tiles over capacity = **0** for both hourglass
and U-tube, both before and after (worst fill in a falling-dominated tile 0.978-0.980 throughout).

`diag_support 512 400`: **0 of 9,712** free-falling cells carry nonzero pressure (after). The task
brief's baseline was "0 of 9,883" — the small difference (9,883 vs 9,712) is scenario/tick-timing
noise between runs of the same stochastic-ish scenario, not a regression; the figure that matters,
**0**, is identical before and after. **Criterion 4 still reads zero. Confirmed.**

### 5. Settled churn — NOT measured (see §2's note; same killed `diag_coarse_ab` run)

### 6. ms/tick — NOT measured (see §2's note; same killed `diag_coarse_ab` run)

## What is unfinished

- **Coupling-strength numbers (measurement 2), settled churn (5), and ms/tick (6) were not
  re-measured after the fix.** `diag_coarse_ab` (`cargo run --release --example diag_coarse_ab`,
  no args, runs all four sub-measurements) takes several minutes per side and was killed mid-run by
  the priority change. Given the saturation-count finding above (transport is clamp-bound, not
  potential-bound), there is reason to expect the pool-levelling/drain-rate numbers did NOT improve
  much even with smoothing — but this is a prediction, not a measurement, and should be verified
  before anyone relies on it.
- **The ms/tick cost of the now-universal bilinear interpolation was not measured.** Every
  coarse-coupled fine edge now does ~8 array reads and NaN checks instead of one lookup-or-zero, on
  an ~8x larger population of edges than before. This could be the dominant new cost; unknown.
- **The I4 budget-attribution approximation** (home-tile spender, not the actual interpolated
  4-corner blend) is documented as deliberate in `coarse_tile_indices`'s doc comment but its
  quantitative accuracy was not separately measured beyond the clamp-count numbers above.
- The new diagnostic `sandart-sim/examples/diag_smooth_eta_banding.rs` is left in the tree
  (uncommitted, like everything else here) — it is a clean, reusable instrument for whoever
  verifies banding again once sub-stepping lands and the coupling flag is reconsidered.

## Handoff state

`sandart-sim/src/coarse.rs` and `sandart-sim/src/physics.rs`: **applied, complete, green** — not
reverted. The change is not mid-flight: it compiles, matches the baseline lib-test count exactly,
passes all seven integration suites, and passes the wasm check, re-verified immediately before this
handoff. `sandart-sim/src/lib.rs`'s uncommitted diff belongs to the concurrent overlay agent, not
this work. Nothing was committed, per instruction.
