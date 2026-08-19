# Step 3 adaptive coarse level — sweep count and incremental restriction

Written against the committed `f670cd48` tree (`artifacts/design/HIERARCHICAL-PRESSURE-PROGRESS.md`,
`artifacts/design/STEP3-FIXES.md`), addressing the ~80% per-tick overhead that tree left open:
"`COARSE_DEFAULT_SWEEPS = 16` is an iteration count a previous agent picked with no justification."

The user's directive: *"what is 16? we only need one coarse simulation per tick. and [the]
simulation should be adaptive."* Two changes, made and measured separately, then together:

1. **The coarse relax is a simulation, not a solver run to convergence.** One sweep is mandatory
   per tick (`M` persists across ticks per §0.4 — "the memory lives in `M`" — so it only needs to
   advance, not converge). Additional sweeps, up to a ceiling of 8 (the design's own §8 budget
   number, not invented here), are spent **per-region**, only on coarse tiles with outstanding
   coarse-fine disagreement `|Delta| = |M-A|` (G1's clock signal — it vanishes at rest, unlike
   `grad P`).
2. **Incremental restriction.** `restrict` only re-aggregates coarse tiles whose block was
   simulated or received flux this tick (`settle_tick`'s existing `modified` machinery,
   `activate_neighbor`/`flux_edge_apply`). A block absent from that set has bit-identically the
   same fine heights it had last time, so re-summing it is a provable no-op. Verified exact by a
   new test, not assumed.

All numbers below are grid 512, release build, inside `distrobox enter sandart-dev`.

---

## 1. Change 1 — adaptive coarse relax

### What changed

`sandart-sim/src/coarse.rs`:
- `COARSE_DEFAULT_SWEEPS`: `16` → `8` (the design's own already-derived ceiling: "`8*4096/262144 =
  12.5%` of one fine sweep", `HIERARCHICAL-PRESSURE.md` §8 — not a fresh constant).
- `CoarseState::relax`: sweep 1 always runs, full grid. Sweeps 2..=8 run only if at least one tile
  is "active" — `|M[C]-A[C]| > capacity[C] * 1e-4` (the same relative tolerance
  `restriction_preserves_total_mass`, this file's own existing test, already uses for floating
  aggregation noise) — and touch only edges where at least one incident tile is active. The active
  set is recomputed after every sweep (cheap: `O(coarse_n^2)` = 4096, not `O(grid^2)`), so a region
  that finishes mid-tick stops earning extra sweeps immediately. If no tile is active, the loop
  exits early.

### Why 1 sweep is defensible, not just cheaper

The design's own §10 says a 64-cell coarse chain needs ~4000 sweeps to fully settle (nonlinear
diffusion, `O(N^2)`) and that settling instead happens over ~500 ticks via persistence regardless
of intra-tick sweep count. So no affordable per-tick `N` reaches convergence within a tick; the
previous `N=16` was mostly spending sweeps that could not do what they were bought for.

### Measured (ms/tick, grid 512, Hourglass, `diag_blocks --ticks 200`)

| configuration | Water | DrySand |
|---|---:|---:|
| `N=16`, full restrict every tick (original tree; reproduced exactly, see §4) | 37.95 | — |
| Change 1 alone: adaptive relax (ceiling 8), full restrict every tick | 30.65 | 23.36 |
| Change 1, literal floor (`N=1`, no adaptive extra ever), full restrict | 23.73 | — |

Sweep-count reduction alone (`N=16`→adaptive ceiling 8) accounts for ~7.3 ms of the ~7.75 ms total
improvement measured for Change 1; the rest (§2) is incremental restriction.

### Behaviour: what one sweep still achieves (`diag_resolution --ticks 400`)

Every metric there is a fraction-of-domain rate, so resolution-invariant physics gives the same
number at every grid size.

| metric (grid 512) | `N=16` (reproduced original) | adaptive (ceiling 8, shipped) |
|---|---:|---:|
| Hourglass descent/tick | 3.866e-5 | 3.902e-5 |
| Pile (pool-levelling) fraction left after 1600 ticks | 0.606 | 0.597 |
| Pile ticks-to-50% (grid 128, for reference) | 181 | 176 |
| Pile ticks-to-50% (grid 256, for reference) | 620 | 595 |

**Statistically indistinguishable from `N=16`, if anything marginally better** (within run-to-run
noise — these are not bit-identical runs, since fewer sweeps changes floating-point history). This
confirms §10's own prediction: pool-levelling and hourglass-drain rate are dominated by cross-tick
persistence, not intra-tick sweep count, so `N=16` was buying almost nothing physically while
costing ~7ms/tick.

`overfill_pressure_toggle` (riser rise, U-tube equilibration, angle-of-repose preservation): all
still pass — see §3.

### Starvation check

By construction, a tile only fails to earn extra sweeps when `|Delta|` is at or below the noise
floor — i.e. the rule cannot deny sweeps to a tile with genuine outstanding disagreement; that IS
the activation condition. Empirically, the pool-levelling and hourglass-drain numbers above show no
degradation from `N=16`, which is the outcome starvation would produce (a region unable to earn work
would lag, not match). No starvation observed.

---

## 2. Change 2 — incremental restriction

### What changed

- `physics.rs::settle_tick` gained an output-only parameter, `touched_out: Option<&mut Vec<bool>>`,
  filled with the tick's final `modified[]` (one bool per block — true iff simulated or flux-marked
  `modified`) just before return. Twenty call sites updated (all test harnesses pass `None`).
- `coarse.rs::CoarseState::restrict_incremental` re-aggregates only tiles flagged `touched`, over
  just that tile's own `t x t` fine-cell footprint (not a full-grid scan). Falls back to a full
  `restrict` if the touched set's length doesn't match (first tick, or after a mask rebuild).
- `lib.rs`: new field `blocks_touched: Vec<bool>`, populated from `settle_tick`'s `touched_out`
  each tick that runs, cleared to all-`false` when no tick runs (nothing could have changed), and
  cleared to empty on `generate_shape_mask` (forces a full rebuild after any geometry change).
  `coarse_state.tick()` — which runs at the top of `update()`, before this tick's own `settle_tick`
  — reads the PREVIOUS tick's `blocks_touched`, matching the heights it actually observes.
- **Block index and coarse tile index are the same integer**: `block_size == grid/64 ==
  COARSE_GRID` at every coupled resolution (both require exact division by 64), so no partial-tile
  bookkeeping is needed — one block is one coarse cell.

### Exactness — verified, not assumed

New test: `coarse::tests::incremental_restrict_is_bit_exact_to_full_rebuild`. Drives the real
production path (`DrawingSimulation::update()`, all ten shipped `SandboxShape`s, grid 128, 300
ticks each) and after every tick recomputes a full `restrict` from a snapshot of the exact fine
state taken immediately before that tick — the same state `coarse_state.tick()` itself reads —
asserting `a_mass`/`support_mass` match the incremental result with `==` (not a tolerance). **Pass,
all shapes, all ticks.**

### Measured cost (isolated from Change 1 by holding the relax ceiling fixed)

| scenario | full restrict every tick | incremental restrict | delta |
|---|---:|---:|---:|
| Hourglass (`diag_blocks`, ceiling 8) | 30.65 ms | 30.13–30.20 ms | ~0.5 ms |
| Hourglass (`diag_blocks`, ceiling 1) | 23.73 ms | 23.40 ms | ~0.3 ms |
| Settled pool (3000-tick warmup, `--settled`) | 25.21 ms | 24.19 ms | ~1.0 ms |

Average touched-block fraction, same runs: Hourglass ~24.9% of 4096 tiles/tick (vs. ~20.6%
`will_simulate`); settled pool ~22.5% of 4096 tiles/tick, even 3000 ticks after the pool stopped
visibly moving (the coarse pressure term keeps a low but nonzero level of fine-cell activity near
the base of any resting column — expected, not a bug, per §0.4's memory argument).

**Honest finding: the saving is real but smaller than the touched fraction alone would predict** —
skipping ~75-80% of tiles saves roughly 1-3% of total tick time, not the same fraction of
`restrict`'s isolated cost. `restrict`'s own full-grid pass is cheap relative to everything else at
this ceiling (adaptive relax, the fine solver itself), so most of Change 1's ~7ms saving is not
recovered a second time by Change 2 in these benchmarks. Incremental restriction is still shipped
because it is a strict improvement with a proven-exact result and zero behavioural risk, but it is
not, on its own, a large lever at the current relax cost — that may change if a future pass makes
the relax cheaper still, at which point `restrict`'s fixed cost becomes proportionally larger again.

---

## 3. Combined (both changes, shipped state)

### ms/tick, grid 512, `diag_blocks --ticks 200`

| | Water | DrySand |
|---|---:|---:|
| Before (original tree, `N=16`, full restrict) | 37.9 | — |
| After (adaptive ceiling 8 + incremental restrict, shipped) | **30.1–30.2** | **22.9–23.0** |
| Uncoupled baseline (given) | ~21 | — |

**15%-of-tick budget: not met, but the gap is much smaller.** Overhead over the ~21 ms uncoupled
baseline: (30.15-21)/21 ≈ **43%**, down from ~80% at `N=16`. The design's 15% ceiling (~24.15 ms)
would require the literal-floor configuration (`N=1`, no adaptive extra): measured **23.40–23.73
ms**, i.e. **~11-13% overhead — inside the 15% budget** — at the cost of the artifact degradation
in §4 below. The shipped ceiling-8 default trades some of that budget headroom for closer tracking
of the `N=16` behaviour in §4's numbers; this is a real, open tradeoff, not a solved one.

### `overfill_pressure_toggle` wall clock

**43.4-43.5 s** (down from 95.15 s after the LUT fix alone, and 95.12 s after all three
STEP3-FIXES defects; uncoupled baseline was 7-8 s). All 7 non-`#[ignore]`d tests in that suite pass,
including `spec_task70_u_tube_riser_keeps_rising`,
`overfill_pressure_u_tube_communicating_vessels_equilibrates`, and
`overfill_pressure_granular_preserves_angle_of_repose` — the riser still rises, the U-tube still
equilibrates, dry sand's angle of repose is still preserved.

### Full verification

- `cargo test -p sandart-sim --lib --release`: **102 passed / 10 failed** (one more pass than the
  101/10 baseline — the new `incremental_restrict_is_bit_exact_to_full_rebuild` test — same ten
  failing names, unchanged; `test_water_blob_stays_left_right_symmetric_under_gravity` still fails,
  on purpose, untouched).
- All six integration suites pass: `overfill_pressure_toggle`, `perfect_simulation_determinism`,
  `fresh_pressure_field_toggle`, `pressure_heatmap_head_field_toggle`,
  `head_field_transport_toggle`, `pressure_sensitive_flow_toggle`.
- `cargo test -p sandart-render --release`: pass.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`: pass.

---

## 4. The pre-existing over-drive artifact — reported, not touched

Flagged mid-session: the coupling over-drives vertical inter-tile edges by roughly `t * base_head`
(`eta[c] = phi - cy * base_head_coarse` uses a fill FRACTION, not a compressed hydrostatic head, so
two similarly-filled adjacent tiles see `delta_eta ~= +8` at grid 512 — nine times gravity on that
edge). This is a pre-existing design gap (§0.2 vs §0.3 disagreement on whether the fine cell reads
`eta[tile]` or `eta[tile] - z_fine`), **not something this session introduced or fixed**, and
**reducing coarse sweeps moves it in the wrong direction**: more sweeps flatten `eta` and shrink
`delta_eta`; fewer sweeps leave it larger. Measured with the same methodology STEP3-FIXES.md used
(`diag_support`'s free-falling-cells-carrying-pressure %, and the `coarse_head != 0` bang-bang fire
count), reproducing the original `N=16` baseline exactly first to confirm the isolation methodology
(298,055 / 87,341 fires match STEP3-FIXES.md's own numbers bit for bit):

| configuration | Hourglass falling-cells-w/-pressure | Hourglass bang-bang/tick | U-Tube falling-cells-w/-pressure | U-Tube bang-bang/tick |
|---|---:|---:|---:|---:|
| `N=16` (reproduced original, full restrict) | 8.4% (852/10195) | 745.1 | 7.7% (1893/24617) | 218.4 |
| Shipped (adaptive ceiling 8 + incremental restrict) | 9.4% (982/10479) | 761.1 | 11.3% (2989/26562) | 198.7 |
| Literal floor (`N=1`, no adaptive extra, + incremental restrict) | 16.9% (2243/13266) | 766.3 | 17.2% (4544/26480) | 123.4 |

**This is the single most useful thing to report: going to fewer sweeps measurably worsens the
existing over-drive artifact, roughly doubling the free-falling-cells-carrying-pressure percentage
at the literal one-sweep floor** (8.4%→16.9% hourglass, 7.7%→17.2% U-tube), consistent with the
mechanism described above (less-converged `eta` leaves larger `delta_eta` spikes at moving fronts).
The bang-bang fire count moves more ambiguously (up slightly for Hourglass, down for U-Tube at
lower sweep counts) — it counts saturation events, a different quantity from the pressure-leak
percentage, and is not a clean proxy for it here.

The shipped ceiling-8 default was chosen for cost reasons (the design's own §8 budget number)
**before** these artifact numbers were measured, not tuned against them — no clamp, scale, or gate
was added anywhere to hide or compensate for the over-drive. It sits closer to the `N=16` numbers
than the literal floor does (9.4%/11.3% vs 16.9%/17.2%), which is a side effect of spending more
sweeps in genuinely-active regions, not a fix. The over-drive itself needs a design-level fix
(§0.2/§0.3's `eta` definition), which is out of this task's scope per instruction.

---

## 5. Files changed

- `sandart-sim/src/coarse.rs` — adaptive `relax`/`relax_sweep`, `restrict_incremental`,
  `COARSE_DEFAULT_SWEEPS` (16→8) and its derivation, `ACTIVE_DELTA_REL_TOL`, `tick`'s new `touched`
  parameter, the new bit-exactness test.
- `sandart-sim/src/physics.rs` — `settle_tick`'s new `touched_out` output parameter; 20 call sites
  updated (all test harnesses pass `None`).
- `sandart-sim/src/lib.rs` — new `blocks_touched` field and its lifecycle (populate, clear-on-idle,
  clear-on-rebuild); `coarse_state.tick()`/`settle_tick()` call sites updated.
- `sandart-sim/src/task55_head_spec.rs` — one missed `settle_tick` call site (own test harness).
- `sandart-sim/examples/diag_step3_adaptive_probe.rs` — new diagnostic used for this report's §2/§4
  measurements (touched-block fraction, settled-pool cost, `diag_support`+bang-bang together).
