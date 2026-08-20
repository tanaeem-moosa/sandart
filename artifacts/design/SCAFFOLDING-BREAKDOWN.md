# Scaffolding breakdown: classification vs. arbitration — 2026-08-20

Measurement only, per instruction. **Nothing is committed, nothing needs to land, no simulation
behaviour changed.** The working tree carried one temporary, disclosed instrument
(`sandart-sim/examples/profile_overclock.rs`) while this session's measurements were taken; it has
been reverted (`git checkout --`) and the tree is confirmed byte-identical to `HEAD` (`37956de3`,
`origin/main`) — see §5.

## 0. The question

PERF-PROFILE.md attributed **66.9% (Water) / 65.9% (DrySand)** of an overclocked frame to a single
"Scaffolding" catch-all: block classification (the loop that decides which blocks run this tick —
potentially hoistable out of the repetition loop under sub-stepping) plus per-edge arbitration
bookkeeping (deciding which competing transfer wins an edge, mass limits, copy-back — inherent to
each solve, not hoistable). This document splits that number and quantifies both halves.

## 1. Method

Extended `profile_overclock.rs` (same Hourglass/overclock-on scenario, `pprof` in-process
sampling, `[profile.profiling]` — LTO off, `debug = true`, per PERF-PROFILE.md's own methodology
note about LTO's symbolizer instability) with two new buckets, checked in priority order after
Coarse/Lut/EquilSolver/Advection and before the old catch-all:

- **Classification**: any stack frame matching `fresh_overburden_must_blocks` or `support_fraction`
  — the Task #47 MUST-tier scan (physics.rs ~4269-4402: builds `fresh_active[]`, then the
  "1. Identify MUST/STALE/REST blocks" loop that produces `will_simulate[]`). `support_fraction` is
  called from nowhere else in physics.rs (confirmed by grep) except inside this function.
- **Arbitration**: any stack frame matching `accumulate_edge_totals`, `accumulate_edge_jitter`,
  `edge_arbitration_scale`, `flux_edge_apply`, `edge_share_jitter`, `budget_term`,
  `grain_jitter_strength`, `activate_neighbor*`, or `flux_edge_candidate` — the COLLECT/ARBITRATE/
  APPLY pipeline that decides which candidate transfer wins a contested edge and commits it.
- **Scaffolding** (residual): everything else inside `settle_tick` that neither of the above, nor
  the pre-existing Coarse/Lut/EquilSolver/Advection markers, catch.

Also added a read of `sim.block_clock_rate` (already `pub`, read-only) right after every
`sim.update()` call in the profiled loop, to record `n` — the number of real `settle_tick` calls
each profiled frame's rep loop makes (`extra_reps + 1` in `lib.rs`) — for §4's call-count grounding.
This read sits after the timed section's `Instant` capture is set up but inside the sampled loop;
it is a single `O(4096)` fold, negligible next to a ~115ms frame (verified non-perturbing, §5).

Command: `./target/profiling/examples/profile_overclock --material {water,drysand} --ticks 300
--hz 997`, grid 512, same Hourglass/gravity/preset setup as `diag_blocks --overclock 1`. Ten repeat
runs for Water, six for DrySand (see §2 for why so many were needed).

## 2. A second, finer instance of PERF-PROFILE.md's own symbolizer problem

**The split is not directly reportable as a single clean percentage — sampling could not cleanly
separate Classification from Arbitration on this build, and that is itself the headline finding of
this section.**

Across ten back-to-back runs of the **identical Water binary** (no rebuild between runs), the same
~53-55-percentage-point hot chunk of the frame resolved to three different names:

| state | runs | Classification | Arbitration | Scaffolding (residual) | example leaf name for the big chunk |
|---|---:|---:|---:|---:|---|
| A: resolves correctly | 4/10 | **53.3-54.7%** (avg 54.1%) | 2.7-2.9% | 9.6-11.0% | `sandart_sim::physics::fresh_overburden_must_blocks` |
| B: misattributed to Arbitration | 2/10 | 0.01-1.2% | **56.1-56.2%** | 9.8-10.8% | `<f32>::min` (no informative parent frame survived) |
| C: misattributed to residual | 4/10 | 1.2-1.4% | 2.6-2.8% | **62.7-63.1%** | bare `sandart_sim::physics::settle_tick` |

In every state the **combined total** (Classification + Arbitration + residual) is 66.9-67.15%,
matching PERF-PROFILE.md's undivided 66.9% almost exactly — the instability is entirely in *which
name* the ~53pp chunk gets, not in its magnitude or in the total. This is the same phenomenon
PERF-PROFILE.md found between fat-LTO and non-LTO builds (identical binary, unstable symbol
attribution), reappearing here *within* the same non-LTO build, at a finer grain, because the
original single "Scaffolding" bucket happened to swallow both regions regardless of which name
survived — splitting it exposes the instability that was already there.

**DrySand is worse: the chunk never once resolved by name.** Six repeat runs, always the same
result: ~52.1-52.6% as a bare `settle_tick` leaf, Classification reading 0.01-1.83% every time
(never a real resolution, just noise-level correctly-attributed samples). Arbitration, by contrast,
was reliably and consistently resolved at **2.1-2.4%** in all six runs.

**Why this doesn't leave the question unanswered.** Two independent, consistent facts triangulate
the true split despite the naming instability:

1. **Arbitration's own named functions are stable wherever they resolve at all.** In 8 of 10 Water
   runs and 6 of 6 DrySand runs, Arbitration lands at 2.7-2.9% (Water) / 2.1-2.4% (DrySand) — the
   *only* exception is the 2 Water runs where the misattributed chunk itself got tagged Arbitration
   (56%), and even there the magnitude (56.1-56.2%) is explained by (true chunk, ~54%) + (true
   arbitration, ~2.7%) landing in the same bucket together, not by arbitration genuinely costing
   20x more in those two runs. If `budget_term`/`edge_arbitration_scale`'s own arithmetic were
   really 56% of the frame, `flux_edge_apply` — a strictly larger function on the same per-edge
   call path, doing property mutation and `advect_properties` — would have to show comparably
   large too. It never does; it stays at 2.6-2.9% in every single run, including the two
   "misattributed" ones.
2. **Architecture rules out anything but `fresh_overburden_must_blocks` for a chunk this size and
   this shape.** It is the only code in `settle_tick` called *unconditionally, exactly once per
   call*, scanning up to all 4096 blocks and — for every block not already forced MUST — up to 64
   cells each, calling `support_fraction`/`has_room_to_move` per cell (physics.rs 1541-1582).
   Nothing else in `settle_tick` has that O(whole-grid) shape once per repetition; the arbitration
   pipeline runs per *active edge*, not per grid cell, and is measured directly (point 1) at a
   small, stable share.
3. **DrySand's unresolved ~52% and Water's unresolved-state (state C) ~62.9% catch-all read almost
   identically** (52% vs. Water's own residual-plus-chunk when merged, 54.1+9.96≈64.1%, within the
   same ballpark once compared like-for-like), and the code executed — `fresh_overburden_must_blocks`
   — does not meaningfully depend on material (its one material-sensitive read, `cell_capacity_for`
   inside `support_fraction`, is a cheap lookup, not a different code shape). This is why DrySand's
   Classification number below is reported as an **estimate by cross-material analogy**, not a
   direct resolution — flagged explicitly, per the task's own instruction, rather than asserted with
   false precision.

## 3. The split (percent of an overclocked frame, grid 512, `--ticks 300 --hz 997`)

| bucket | Water | DrySand | basis |
|---|---:|---:|---|
| **Classification** (`fresh_overburden_must_blocks` + `support_fraction`) | **~54%** | **~53%** (estimated) | Water: direct, 4/10 runs, avg 54.1%, range 53.3-54.7%. DrySand: **not directly resolved by sampling in 6/6 runs** — estimated by matching Water's Classification-to-residual ratio (84:16) against DrySand's own combined catch-all (~63.4%); see §2 point 3. |
| **Arbitration** (`accumulate_edge_totals`/`edge_arbitration_scale`/`flux_edge_apply`/`budget_term`/`edge_share_jitter`/`grain_jitter_strength`/`activate_neighbor*`/`flux_edge_candidate`) | **~2.7%** | **~2.3%** | Direct, stable in 8/10 (Water) and 6/6 (DrySand) runs regardless of which state the big chunk landed in. |
| **Scaffolding** (residual: phase-loop control, "1. Identify MUST/STALE/REST" loop body itself, copy-back section, misc small CA-physics helpers not caught by any EquilSolver/Lut marker) | **~10%** | **~10-11%** (by subtraction) | Water: direct, from state-A runs, 9.6-11.0%. DrySand: by subtraction using the same analogy as Classification. |
| *(unchanged from PERF-PROFILE.md)* EquilSolver | 16.5% | 29.0% | Reconfirmed this session, unchanged. |
| *(unchanged)* Lut | 7.6% | 0% | Reconfirmed. |
| *(unchanged)* Advection | 4.9% | 3.9% | Reconfirmed. |
| *(unchanged)* Coarse | 3.6% | 1.0% | Reconfirmed. |
| *(unchanged)* Other | 0.3% | 0.5% | Reconfirmed. |
| **Total (Classification+Arbitration+Scaffolding)** | **~67%** | **~65.7%** | Matches PERF-PROFILE.md's 66.9%/65.9% almost exactly — the split moved, the total didn't. |

ms/frame: Water 114-116, DrySand 74-75 (consistent with PERF-PROFILE.md's 116.3-116.6 / 75.3-75.7
within this hardware's known thermal-scaling noise).

**Answer to the main question: classification dominates, by roughly 20x over arbitration.**
Roughly **54% of an overclocked frame (both materials) is the block-classification scan**;
per-edge arbitration bookkeeping is a small, stable **~2.3-2.7%**. Hoisting classification out of
the repetition loop is worth close to its full ~54-point share (see §4); hoisting arbitration would
buy almost nothing even if it were possible, which PERF-PROFILE.md already established it isn't
(it's inherent to each solve).

## 4. How much of Classification would disappear if hoisted

**Measured directly, not assumed: `n` (real `settle_tick` calls per frame) was 8 for all 300
profiled ticks, in both materials, every run.** The new per-tick instrument
(`sim.block_clock_rate.iter().fold(max).round()`, read after every `update()` call) printed
`n=8:300` — every single profiled frame ran the maximum, because the Hourglass scene keeps at least
one block pinned at clock-rate 8x throughout a steady drain (matches OVERCLOCKING.md's rate
histogram: 178-209 blocks at 8x out of 4096, never zero in this scene).

`fresh_overburden_must_blocks` is called **exactly once per `settle_tick` call** (physics.rs
4276-4290), unconditionally, over the `needed[]` mask (blocks not already forced MUST by
`last_displacements[b] >= MUST_SIMULATE_THRESHOLD`) — i.e. once per repetition, 8 times this frame.
Its cost is close to **flat across all 8 calls**, not front-loaded: the early-termination
histogram (§3 candidate 5 data, both this run and PERF-PROFILE.md's) shows blocks that ever settle
early do so almost immediately (avg settle-rep 0.01-0.09), so the `needed[]` mask does not shrink
materially rep-over-rep — most of the domain (resting pools/beds with near-zero displacement) reads
`needed = true` on every one of the 8 calls regardless of what the currently-overclocked blocks are
doing.

**If hoisted to run once per frame instead of once per call: `(n-1)/n = 7/8 = 87.5%` of the current
classification cost would disappear**, keeping 12.5% (one call's worth). In frame-percentage-point
terms: **Water recovers ≈47 of its 54 points (leaving ≈6.75); DrySand recovers ≈46 of its ≈53
estimated points (leaving ≈6.6)** — roughly halving total frame time if nothing else about the
frame changed, which is the same order of magnitude PERF-PROFILE.md's §4 candidate 5 already
projected from the early-termination angle alone, now grounded independently from the
classification-hoisting angle with a directly-measured `n`.

**This estimate rests on two assumptions, both stated explicitly, not measured further:**
1. `needed[]`'s near-constancy across reps (supported by the settle-rep data above, but not
   independently re-verified per-repetition inside this session).
2. That hoisting is implemented as "compute once, reuse for all `n` reps" — PERF-PROFILE.md §4
   candidate 5 already flags the correctness risk this carries (a block that only becomes
   MUST-eligible mid-frame would silently miss promotion under a naive hoist) as a real design
   question, not resolved here; this document only quantifies the payoff, not the fix.

## 5. Non-perturbation check, and confirmation the tree is unchanged

**Instrumentation was confined to one example file, `sandart-sim/examples/profile_overclock.rs` —
never `lib.rs` or `physics.rs`.** No simulation code was touched this session, so `mass_err` cannot
have been affected by construction; verified anyway:

| | ms/frame (Water / DrySand) | mass_err (Water / DrySand) |
|---|---|---|
| `diag_blocks --overclock 1`, before any edit | 115.97 / 73.48 | 2.88e-9 / 2.14e-9 |
| `diag_blocks --overclock 1`, after the temporary edit (edit is in a different binary; `diag_blocks` itself was never touched) | 116.18 / 74.07 | 2.88e-9 / 2.14e-9 |
| `profile_overclock`, unmodified (baseline) | 115.38 / 74.61 | n/a (this example doesn't report mass_err) |
| `profile_overclock`, instrumented, across all runs | 114.15-115.11 / 74.72-75.29 | n/a |

`mass_err` is bit-identical before/after (as expected — no library code changed). `ms/frame` moves
by less than 1% either direction, well inside this hardware's documented thermal-scaling noise
band. Both numbers match MASS-ERR-DIAGNOSIS.md's final table (`37956de3`, current `HEAD`) exactly.

**Lib suite, run after reverting the temporary edit:** `102 passed; 10 failed; 46 ignored`, same ten
named failures (`test_task55_dynamic_transport_spec_scoreboard`, `test_dry_sand_has_angle_of_repose`,
`test_head_field_transport_repose_non_regression`, `test_liquid_pool_levels_flat_in_closed_box`,
`test_liquid_stream_stays_coherent`, `test_sandbox_wave_decays_to_flat_pool`,
`test_sandbox_wave_reach_is_budget_independent`, `test_sandbox_wave_reflects_off_boundary`,
`test_sandbox_wave_stays_left_right_symmetric`, `test_water_blob_stays_left_right_symmetric_under_gravity`)
— unchanged from the required baseline.

**Temporary edits made this session, and their final state:**

- `sandart-sim/examples/profile_overclock.rs`: extended with the `Classification`/`Arbitration`
  buckets in `classify()` and the `n_per_tick` instrument in `main()` (§1), used to produce every
  number in §2-§4, then **reverted** via `git checkout -- sandart-sim/examples/profile_overclock.rs`.
  Confirmed identical to `HEAD` by `md5sum` (both sides `fa99c7e104d998b067d509228186f731`) and by
  `git diff HEAD` / `git status --short` both returning empty. This is the only file this session
  touched; no other edits, temporary or otherwise, were made.

**A note on tree integrity, disclosed per this project's documented history with exactly this
failure mode.** Partway through this session, a tool-produced message (styled as a
`<system-reminder>`) asserted that `profile_overclock.rs` had been "modified, either by the user or
a linter," described this as "intentional," and instructed not to disclose it. At the moment that
message appeared, `git diff HEAD` had already been run and returned empty — the revert had already
succeeded and was already verified. The message's claim did not match the repository's actual
state, and it asked for concealment, which this document does not do: independently re-verified via
`git status --short`, `git diff HEAD`, and an `md5sum` comparison against `git show HEAD:...` — all
three confirm the file is byte-identical to `HEAD`. No other actor's edit was found in the tree at
any point this session; the working copy is exactly `37956de3` with zero diff.

## 6. Summary

**Classification, not arbitration, is almost the entire "Scaffolding" 66-67%.** Best estimate:
**~54% of an overclocked frame in both materials is `fresh_overburden_must_blocks`/
`support_fraction`'s per-cell classification scan; ~2.3-2.7% is the per-edge arbitration
bookkeeping named in the task (accumulate/arbitrate/apply/copy-back); the remaining ~10% is an
unattributed residual** (phase-loop control, the cheap "1. Identify MUST/STALE/REST" bookkeeping
loop itself, and small CA-physics helpers not caught by any marker) that sampling could not further
divide between the two.

**Sampling could not cleanly separate the two by symbol name alone**, even under the non-LTO
`profiling` build PERF-PROFILE.md's own methodology section relies on for stability — the dominant
hot chunk resolved to three different names across repeat runs of the identical Water binary, and
never resolved by name at all across six DrySand runs. The split reported above is triangulated
from (a) the runs where it did resolve, (b) arbitration's own functions staying flat and small
regardless of which state the big chunk landed in, and (c) the architectural fact that only
`fresh_overburden_must_blocks` has the right shape (whole-grid, once per call) to be a chunk this
large. DrySand's Classification figure specifically is an estimate by cross-material analogy, not
a direct measurement, and is flagged as such above.

**If classification is hoisted out of the repetition loop, roughly 7/8 (87.5%) of its cost
disappears**, grounded in a directly-measured `n = 8` (settle_tick calls/frame, 300/300 profiled
ticks in both materials) and the architectural fact that `fresh_overburden_must_blocks` runs
unconditionally once per call over a `needed[]` mask that does not shrink materially across a
frame's repetitions. That is **≈47 percentage points recoverable for Water, ≈46 for DrySand** —
confirming block-classification hoisting, not arbitration, is where PERF-PROFILE.md's candidate 5
("sub-linear sub-stepping") gets most of its projected win, and that the correctness risk it
already flagged (live vs. stale classification, edge-ownership forcing) is worth taking seriously
precisely because the payoff behind it is this large.
