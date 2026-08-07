# Task #55 — elliptic solve cost, scoping, and the fresh_pressure_field composition question

All measurements at w=512 unless stated otherwise. Tree: started at `9f38d31` (96 passed / 1
known-failing / 34 ignored). Only file touched: `sandart-sim/src/physics.rs`. Nothing committed.

## Bottom line (read this first)

1. **The elliptic gate is ~45x slower than shipped baseline, unchanged by scoping.** That is
   past the "must be scoped before it can ship" line (~100x), not past the "sluggish but usable"
   line (~10x) — it sits in between, closer to the bad end. Scoping made it *exact* (proven
   bit-identical) and cut its overhead on liquid-free ticks by ~61%, but did **not** move the
   number that matters for the ship/no-ship call, because the benchmark scenario (and most
   interesting real scenarios — a body of liquid actually settling) is dominated by the solve
   itself, not the full-grid setup this task could safely scope. See Job 1/2 below for why, in
   detail. **Recommendation: do not ship the elliptic gate yet, even as a default-off debug
   toggle**, unless "debug toggle, rarely flipped, user warned it may hitch" is an acceptable bar
   — at ~45x, a 512-wide sandbox with any real body of water goes from ~3.4 ms/tick to ~150+
   ms/tick, i.e. from smooth to a multi-second-per-frame stall the instant it's flipped on.

2. **`fresh_pressure_field` can safely become the default.** Its own cost is cheap (~1.4x, a
   single extra full-grid pass), the elliptic solve does NOT fully cancel the arch regression it
   causes when both are on, but it also does not make it worse — on the arch scenario the ON/ON
   cell's peak overshoot and final result are both *better* than fresh_pressure_field-alone, and
   better than shipped baseline on the metric that matters (final unsupported mass). See Job 3.

---

## Job 1 — elliptic solve cost, before scoping

**Method.** `elliptic_head_gate::is_enabled()`'s `#[cfg(not(test))]` twin
(`sandart-sim/src/physics.rs`, currently lines ~2407–2415) hardcodes `false` for every non-test
build, including `bench_sandfall`. To reach the gate from that binary I temporarily edited that
twin to return `true`, rebuilt, measured, then reverted it back to `false` and rebuilt again to
confirm the revert. **This edit is not present in the tree I am leaving behind** — verified by
diffing the current `elliptic_head_gate` block byte-for-byte against `git show
HEAD:sandart-sim/src/physics.rs`; they match exactly.

**Command:** `cargo run --release --example bench_sandfall -- --ticks 60 --materials water
--budgets 1024 --reps 3 --warmup 5` (w=512, Hourglass, `--reps 3` per the bench's own
methodology — reports the minimum of 3 passes to filter Steam Deck thermal/frequency noise).

| config | ms/tick |
|---|---|
| gate OFF (shipped default) | 3.4367 |
| gate ON, **unscoped** (original `elliptic_liquid_level_pass`) | 153.3458 |

**Multiplier before scoping: 153.3458 / 3.4367 ≈ 44.6x.**

The prior agent's report of ">12 minutes, no number" is explained, not contradicted: the
default `bench_sandfall` invocation (no flags) runs 2 materials × 3 budgets × 3 reps × 900 ticks
≈ 16,200 timed ticks plus warmup. At ~145–155 ms/tick that is roughly 40 minutes — a 12-minute
window was never going to finish it. The short, single-budget, single-material, 60-tick
invocation above is what actually produces a number in about 30 seconds.

## Job 2 — scoping, and the number after

**What was scoped.** `elliptic_liquid_level_pass` ran four unconditional full-grid (`w*h`)
passes every tick the gate was on: (1) build the liquid-domain node set, (2)
`recompute_column_depth` (an unrelated top-down accumulation, shared with `fresh_pressure_field`
— **left untouched**), (3) per-node conveyance/capacity, (4) build the edge list. Only step 1 is
truly unavoidable (no per-block "contains liquid" cache exists anywhere in
physics.rs/lib.rs — confirmed by a targeted search before writing anything). Changes made,
in `sandart-sim/src/physics.rs`:

- Step 1 now also records, for free (same loop, no extra pass): `participating_columns[x]` (does
  column `x` contain any domain node) and `max_domain_row` (deepest row any domain node occupies).
- **Early return before step 2** when fewer than 2 candidate nodes exist — "nothing is out of
  equilibrium" is trivially true when no edge (an adjacent pair) can exist. This is the literal
  reading of the task's "cheap pre-check to skip the whole pass" — it is exact, not a heuristic:
  it returns exactly what the old code already returned in this case (`edges.is_empty()`), just
  before paying for the passes that would have discovered that.
- **New `recompute_column_depth_scoped`** (a separate function — `recompute_column_depth` itself
  is untouched, so `fresh_pressure_field`'s only other caller of that function is provably
  unaffected by anything in this task). Same formula line-for-line, restricted to
  `participating_columns` and rows `1..=max_domain_row`. This restriction is **exact, not
  approximate**: the formula only ever reads `column_depth` at `center_idx - w` (same column, one
  row up) — there is no lateral term — so skipping a column that contains no domain node cannot
  change the value computed for a column that does. Rows are never cropped from the *top* (row 1
  down), because overburden from a non-liquid cell resting above a domain cell must still be
  counted.
- Steps 3 and 4 (conveyance/capacity, edge list) are now bounded to the same
  `participating_columns` × `1..=max_domain_row` rectangle, for the identical reason — `is_node`
  is provably false everywhere outside it.

**Equivalence, shown not asserted.** Ran the pre-existing `diag_task55_elliptic_propagation`
(covers the arch scenario and the pocket scenario, `multiplicative_lateral_gate` on/off, both the
adaptive scheduler and `perfect_sim_tick`, plus `test_dry_sand_has_angle_of_repose` re-run with
the gate forced on) with `--nocapture` twice: once on the original unscoped code (via `git
stash`, gate forced on the same way as Job 1), once on the scoped code. **`diff` of the two
19-line outputs: zero differences — bit-identical.** Full suite also re-run after scoping:
**96 passed / 1 known-failing / 36 ignored** (the 2 extra ignored are new Job-3 diagnostics added
below; ignored-test count is otherwise unchanged) — matches the pre-change baseline exactly, and
`test_dry_sand_has_angle_of_repose` still PASSES with the gate forced on (confirms the
liquid-only domain restriction still holds after scoping).

**Re-measured Job 1's number, same command:**

| config | ms/tick |
|---|---|
| gate OFF | 3.4367 (unchanged — scoping only affects code the gate reaches) |
| gate ON, **scoped** | 154.4779 |

**Multiplier after scoping: 154.4779 / 3.4367 ≈ 45.0x — essentially unchanged (nominally +1%,
within this benchmark's run-to-run noise).**

**Why scoping didn't help here, measured not guessed.** Instrumented `domain_cell_count`,
`participating_columns` count, and `max_domain_row` directly (temporary `eprintln!` behind an
env var, removed before finishing — not in the diff). At tick ~5 of the water-hourglass
benchmark: `domain_cell_count≈46,150` (17.6% of 262,144 total cells), `participating_columns≈
357/512` (~70% of width), `max_domain_row≈258/512` (~50% of height). So the scoped rectangle
still covers roughly a third of the grid, and — more importantly — the dominant cost was never
the full-grid setup this task scoped: it's the Gauss-Seidel relaxation itself, 48 iterations over
~2×46,150 ≈ 92,000 edges, each doing a feasibility-clamped transfer. That cost is (and always
was) proportional to actual domain size, not grid size, so removing full-grid overhead around it
doesn't touch it. Scoping the setup passes was still the correct, safe thing to do (see next
paragraph for where it DOES pay off), but it was never going to close a 44.6x gap dominated by
solver iterations.

**Where scoping DOES pay off, measured:** a liquid-free tick. `bench_sandfall --materials
drysand` (no water at all — `domain_cell_count=0`, confirmed by the same instrumentation), w=512,
budget 1024, reps=3:

| config | ms/tick | elliptic's own added cost |
|---|---|---|
| gate OFF | 17.79 | — |
| gate ON, unscoped | 29.29 | 11.50 ms |
| gate ON, scoped | 22.31 | 4.52 ms |

**~61% reduction in the elliptic pass's own overhead when there's no liquid on screen at all**
(11.50 ms → 4.52 ms) — the early-return plus the one remaining unavoidable full-grid scan (step
1) is what's left. This is a real, exact win for the common case of "gate on, but nothing wet is
currently on screen"; it just isn't the case the ~44–45x number above is measuring, and the task
explicitly asked to report both honestly rather than only the flattering one.

**Recommendation for Job 1/2 together:** ~45x is between "sluggish but usable" (~10x) and "must
be scoped before shipping at all" (~100x) — closer to the latter. Scoping fixed the wasted
full-grid overhead that existed (and that part of the fix is real, exact, and worth keeping
regardless), but the dominant cost — the 48-iteration Gauss-Seidel sweep over the actual liquid
graph — is architectural, not overhead, and this task's scope (per its own brief) did not extend
to changing solver iteration count or algorithm, which would be the only way to move the ~45x
number further. **Do not enable this as a shipped default-off debug toggle without either (a)
reducing `ELLIPTIC_ITERATIONS` / the solver's own per-tick cost, which is out of scope here and
would need its own convergence-behavior review, or (b) accepting that flipping it on a
water-heavy 512 scene will visibly stall the app (~150+ ms/tick, i.e. under 7 fps) for as long as
it's on.**

---

## Job 3 — composition: does elliptic cancel the fresh_pressure_field arch regression?

**Method.** `diag_task55_arch_collapse_rate` and `diag_task55_pocket_equalisation` (both already
resolution-parametric) vary `multiplicative_lateral_gate` × scheduler, not
`fresh_pressure_field` × `elliptic_head_gate` — they don't answer this question as written. Added
two new diagnostics reusing their exact w=512 scenario construction and metrics verbatim
(`diag_task55_composition_arch_w512`, `diag_task55_composition_pockets_w512`, both `#[test]
#[ignore]`, no assertions — reproduce-only, same as the diagnostics they're modelled on), varying
`fresh_pressure_field` (a plain field on the test harness's `TestSim`, no gate hack needed) ×
`elliptic_head_gate` instead. **Adaptive scheduler only** (`sim.tick`, not `perfect_sim_tick`) —
the composition question is "can these two ship together," which is about the scheduler that
actually ships.

**Tick budget, held identical across all four cells of each matrix by construction (one shared
loop, one shared `run_ticks`):** arch used `run_ticks=600`, `budget_n=256` (same numbers
`diag_task55_arch_collapse_rate` already uses at w=512, for comparability); pockets used
`run_ticks=150`, `budget_n=256` (same as `diag_task55_pocket_equalisation` at w=512). Arch took
~570s wall-clock for its 4 cells (600 ticks × 4, at up to ~150+ ms/tick with elliptic on);
pockets took ~8s (much smaller liquid domain — two wells, not a full chamber).

### Pockets (`diag_task55_composition_pockets_w512`, ProceduralFunnel two-well+basin)

| fresh_pressure_field | elliptic_head_gate | ticks_to_halve |
|---|---|---|
| OFF | OFF | 29 (shipped baseline) |
| OFF | ON | **8** (elliptic alone: much faster levelling) |
| ON | OFF | **37** (worse than baseline — the regression, quantified at w=512 for the first time) |
| ON | ON | 33 (better than fresh_pressure-alone's 37, but still worse than baseline's 29, and far worse than elliptic-alone's 8) |

All four cells halved within budget (150 ticks) — none is "never halved."

**Reading:** on pockets, elliptic does **not** cancel the regression — it partially offsets it
(37 → 33) but doesn't get back to baseline (29), let alone to elliptic-alone's own much better
number (8). Combining them is better than `fresh_pressure_field` alone, worse than either
baseline or elliptic alone.

### Arch (`diag_task55_composition_arch_w512`, pillars + suspended slab over a void)

Metric is `unsupported_span` (lower is better; the task's own scenario is known to get WORSE
before it gets better — a hanging body destabilises further before it collapses/settles — so the
question is the size of that rise, not whether one happens).

| fresh_pressure_field | elliptic_head_gate | ticks_to_halve | peak (tick) | final (tick 600) |
|---|---|---|---|---|
| OFF | OFF | 522 (shipped baseline) | 1.5664x (tick 225) | 0.3552x |
| OFF | ON | 466 | 1.5952x (tick 150) | 0.1253x |
| ON | OFF | 545 (worse than baseline — the regression) | 1.6524x (tick 300) | 0.3507x |
| ON | ON | 464 | **1.4883x (tick 225)** | **0.0842x** |

All four cells halved within budget (600 ticks).

**The rise in ON/ON, addressed directly.** `unsupported_span` in the ON/ON cell climbs to 1.4883x
of its initial value by tick 225 before decaying to 0.7225x by tick 450 and 0.0842x by tick 600.
This is **not worse than baseline** — it's the smallest peak of all four cells (baseline peaks at
1.5664x, `fresh_pressure_field`-alone peaks highest at 1.6524x, elliptic-alone peaks at 1.5952x).
Every one of the four configurations shows the same rise-then-decay shape the task said to
expect; ON/ON's rise is the mildest, not the worst, and its final result (0.0842x remaining) is
the best of the four by a wide margin — about 4.2x less unsupported material left at tick 600
than baseline, and about 4.2x less than `fresh_pressure_field` alone.

**Reading:** on arch — unlike pockets — elliptic does more than cancel the regression: ON/ON
beats not only `fresh_pressure_field`-alone (545→464 ticks-to-halve, 0.3507→0.0842 final) but
also beats shipped baseline itself (522→464 ticks-to-halve, 0.3552→0.0842 final) and beats
elliptic-alone too on the final-mass metric (0.1253→0.0842).

### fresh_pressure_field's own perf cost (isolated, elliptic off)

Measured separately with a temporary standalone example (`sandart-sim/examples/
bench_fresh_pressure_tmp.rs`, written, run, then **deleted** — not part of the diff), w=512,
Hourglass, Water, budget 1024, 60 ticks, 5 warmup, min of 3 reps — `fresh_pressure_field` is a
plain `pub` field on `DrawingSimulation`, no gate hack needed for this one:

| config | ms/tick |
|---|---|
| OFF | 3.3610 |
| ON | 4.6758 |

**Multiplier: 1.391x.** Cheap, as expected ("replaces an O(1) inline running sum with a full-grid
pass" — one extra full-grid pass on a 512² grid is inherently inexpensive next to the rest of a
tick).

### Job 3 recommendation

**`fresh_pressure_field` can safely become the default and have its toggle removed.** Its own
cost is negligible (1.39x). The composition picture is mixed but never bad for the user-visible
outcome the change is meant to fix:

- On **pockets**, elliptic (if ever shipped) would only partially offset the regression, not
  cancel it — but `fresh_pressure_field` alone is being promoted to default on its own merits
  (visually confirmed correct by the user), independent of whether elliptic ever ships.
- On **arch** — the scenario `fresh_pressure_field` is specifically known to make worse — turning
  elliptic on alongside it does not merely cancel that regression, it overshoots past baseline:
  ON/ON is better than shipped baseline on every metric measured (ticks-to-halve, peak, final).

Since elliptic itself is not being recommended for shipping yet (Job 1/2's ~45x, unresolved by
this task's scope), this composition result is not an argument for shipping elliptic now — it's
evidence that **if/when elliptic's cost is brought down, turning both on together is a plausible
path to fully closing the arch regression**, which is useful context for that future decision.
For the immediate ask — can `fresh_pressure_field` ship alone as default — the answer is yes: its
own cost is cheap, and nothing measured here found a NEW problem with it beyond the already-known,
already-accepted arch regression.

---

## Constraints checklist

- **Reverted:** the temporary `elliptic_head_gate` `#[cfg(not(test))]`-twin edit used to reach
  the gate from `bench_sandfall` for Job 1/2's measurements. Confirmed by diffing the current
  block against `git show HEAD:sandart-sim/src/physics.rs` — they match exactly, gate is `false`
  in the non-test build path, same as shipped.
- **Deleted:** the temporary `sandart-sim/examples/bench_fresh_pressure_tmp.rs` used for the
  isolated `fresh_pressure_field` cost measurement in Job 3. Not present in `git status`.
- **Suite is green:** `cargo test -p sandart-sim --release` → 96 passed / 1 failed / 36 ignored.
  The 1 failure is `test_water_blob_stays_left_right_symmetric_under_gravity`, the pre-existing
  intentional marker — untouched, unweakened, same scan order. The ignored count grew from 34 to
  36 because this task added two new `#[ignore]`d diagnostics (Job 3's composition tests); no
  existing test was newly ignored. `--test perfect_simulation_determinism` and `--test
  fresh_pressure_field_toggle` both pass (2/2 each).
- **All gates OFF by default** in the tree left behind: `elliptic_head_gate`,
  `multiplicative_lateral_gate`, and `fresh_pressure_field` (still opt-in; this report
  recommends flipping its *default*, but has not changed the code to do so — that's a decision
  for the user, not made unilaterally here) all read `false`/off with no arguments.
- **`pressure_project`, `clamp_edge_feasible`, `support_fraction`,
  `fresh_overburden_must_blocks`, `recompute_column_depth`**: none of these were modified.
  `recompute_column_depth_scoped` is a new, separate function; the original is byte-for-byte
  unchanged (verified via the `diag_task55_elliptic_propagation` bit-identical diff above, which
  exercises `fresh_pressure_field`'s only other call site of `recompute_column_depth`).
- `block_size` / 32×32 tiling: untouched.
- Nothing committed.

## Files touched

- `sandart-sim/src/physics.rs` — Job 2 scoping (`elliptic_liquid_level_pass`, new
  `recompute_column_depth_scoped`), Job 3 diagnostics
  (`diag_task55_composition_arch_w512`, `diag_task55_composition_pockets_w512`). This is the
  **only** modified file (`git status --short` shows nothing else).
