# Early stop and unquantised clock rates — 2026-08-20

Acts on `artifacts/design/PERF-PROFILE.md`'s finding. Written by the main thread; the implementing
agent hit its session limit mid-measurement, so every number here was re-measured directly against
the final tree.

**Note on a methodology trap that nearly produced a wrong conclusion.** The agent left a
`let still_has_work = true; // TEMPORARY ISOLATION TEST` line in
`force_overclocked_blocks_active`, disabling early stop so it could take a "before" reading, and was
killed before restoring it. Measurements taken against that mutating tree mixed the two states —
one `mass_err` reading was attributed to early stop that came from an intermediate state. **Do not
measure a tree an agent is actively editing.** All numbers below are from a stable tree.

## What changed

1. **Early stop.** A block's clock rate is now a **budget, not a mandate**. Eligibility for each
   extra repetition is gated on the block's own physically-computed `last_displacements` — written
   by the *previous* repetition's `settle_tick` — still being at or above `MUST_SIMULATE_THRESHOLD`.
   A block that reached local equilibrium stops consuming repetitions.
   - It is **eligibility only**: a settled block still RECEIVES mass from running neighbours (S2).
   - **Neighbour-forcing (S3) is deliberately NOT gated on it**, so a clock-domain boundary never
     runs at the slow side's rate.
   - A block a neighbour pushes into has its displacement rise back above the bar and becomes
     eligible again on the next repetition. Settled does not mean frozen.
2. **Rates are no longer quantised to powers of two** — an arbitrary value in `[1/8, 8]` from a
   plain continuous rule, `rate = clamp(signal / CLOCK_DELTA_REF_FRAC, ...)`.

### Why dropping powers of two is safe HERE

Design §7b's S1 requires nesting so that clock domains do not beat and a shared edge never sees one
side mid-step. **That does not apply to this implementation.** `update()` repeats whole `settle_tick`
calls over a *participation set*, and every repetition is a global synchronisation point: a block
either runs in that rep or does not, atomically. A rate-3 block sitting out rep 3 while its rate-4
neighbour runs is structurally identical to a rate-2 block sitting out rep 1. There is no partial
per-substep state to protect.

With early stop bounding real repetitions at the block's own settle point, the precise rate value
matters much less anyway — which is why the elaborate octave-stepped rule with hysteresis was
replaced by a continuous one.

## Measured, grid 512, `diag_blocks --ticks 300`

| material | clocking | ms/frame | com descent | blocks run | mass_err |
|---|---|---|---|---|---|
| Water | off | 21.48 | 0.00738 | 406.7 | 2.20e-9 |
| Water | on, before early stop | 121.9 | 0.06142 | 492.0 | 8.95e-10 |
| Water | **on, with early stop** | **90.57** | **0.06031** | **366.0** | **3.75e-8** |
| DrySand | off | 13.93 | 0.03653 | 417.3 | 2.71e-8 |
| DrySand | on, before early stop | 81.88 | 0.07450 | 411.5 | 5.78e-9 |
| DrySand | **on, with early stop** | **61.13** | **0.07311** | **285.4** | **8.11e-10** |

**The win is real and material movement is preserved** — Water 1.35x faster, DrySand 1.34x, with
descent within 2% of the pre-early-stop value. That was the guard that mattered: buying frame time
by moving less material would have bought nothing.

**It is short of the profile's 2-3x estimate.** Blocks run fell 492 -> 366 (Water) and 411 -> 285
(DrySand), so wasted repetitions are genuinely gone; the remaining cost is the ~66% scaffolding
share, which early stop does not touch.

## OPEN: Water's mass_err regressed and is not explained

`mass_err` for Water rose from 8.95e-10 to **3.75e-8**, reproducible to the digit across runs and
across two independent tree states (3.07e-8 in the earlier, partially-reverted tree). DrySand moved
the other way, 5.78e-9 -> 8.11e-10.

Context that keeps it in proportion: 3.75e-8 against ~135,648 total mass is a relative error of
~2.8e-13, and DrySand's *shipped, unclocked* baseline is 2.71e-8 — the same order. So it is small in
absolute terms and comparable to what already ships.

But it is a **deterministic** change in a quantity the design treats as structural (I6: real mass
moves only through the fine edge solver, conservative by construction). A reproducible 17x increase
against the same-material control means something changed that should not have. The most likely
candidate is the one this change was briefed to avoid: a block stopping early while it still owns
edges a running neighbour needs, leaving a transfer half-applied. **Not diagnosed. Do not tune it
down; find it.**

This is why `overclocking_enabled` remains **default OFF**.

## The steps readout

The user asked to see "total of the steps after under and overclocking in the ui". The readout must
show the **executed** count, not the planned one — early stop makes those diverge, and that
divergence is the whole effect being measured. Shown against the block count so the multiplier is
legible, and able to fall below it when underclocked blocks sit out.

## Verification

Lib suite 102 passed / 10 failed, the same ten named failures. All eight integration suites pass
including `overclocking_toggle` and `perfect_simulation_determinism`, plus `sandart-render`, the
wasm32 check, `cargo check -p sandart`, and `node scripts/check_js.js`.

## The max-clock-rate sweep, and what underclocking is worth (2026-08-20)

`diag_blocks --ticks 300 --overclock 1 --material water --maxrate N`, grid 512, continuous rates.
`max_clock_rate` is now a runtime field with a UI slider ("Max clock rate", Debug panel, visible
only while overclocking is on), so this is a knob the user can move, not a constant.

| max rate | ms/frame | fps | descent (300 ticks) | descent per ms of wall clock |
|---|---|---|---|---|
| 1 (= off) | 22.57 |  44 | 0.00746 | 3.31e-4 |
| 2         | 34.07 |  29 | 0.01647 | 4.83e-4 |
| 3         | 47.14 |  21 | 0.02450 | 5.20e-4 |
| 4         | 58.48 |  17 | 0.03192 | 5.46e-4 |
| 6         | 78.95 |  13 | 0.04641 | 5.88e-4 |
| 8         | 97.33 |  10 | 0.06053 | 6.22e-4 |

**Movement per unit wall clock rises monotonically with the ceiling — there is no efficiency peak
to find below 8.** Cutting the ceiling does not make the simulation cheaper per unit of settling;
it makes each frame cheaper and settling proportionally (slightly worse than proportionally)
slower. The scaffolding a frame pays regardless — classification, the coarse level, copy-back — is
amortised over more useful sub-steps at a higher ceiling, which is why the trend runs this way.

So the slider is an interactivity control, not an optimisation: **max 2 buys 29 fps at 2.2x the
settling rate of no clocking at all**, which is the setting to try first if the drain has to look
live. It is NOT the lever that gets 512 to 60 fps with fast settling; that needs the scaffolding
share itself to come down (SESSION-HANDOVER §6 step 2, phase timers).

### Underclocking is inert in this scene

Control run, ceiling held at 8, floor moved from 1/8 to 1.0 (`--minrate 1`, which disables
underclocking without touching overclocking):

| rate range | ms/frame | blocks run | descent |
|---|---|---|---|
| [0.125, 8] (shipped) | 97.25 | 324.7 | 0.06053 |
| [1.0, 8] (no underclocking) | 96.77 | 324.9 | 0.06054 |

**Nothing. 0.5% on frame time, which is inside run-to-run noise, and the descent is identical to
four decimals.** ~3,400 of 4,096 blocks sit below 1x and it buys nothing measurable, for a
mechanical reason visible in the same output line: `budgeted 0.0`. `apply_underclock_skip` can
only keep a block out of the BUDGET tier — it never touches MUST or STALE — and in the Water
hourglass the budget tier is empty every tick. The blocks it defers were not going to run anyway;
the real filter is MUST classification, which is already only admitting ~325 of 4,096.

DrySand does have a non-empty budget tier (`budgeted 26.7`), so it was run as a second scene, and
the answer there is the same:

| rate range | ms/frame | blocks run | must | budgeted | descent |
|---|---|---|---|---|---|
| [0.125, 8] (shipped) | 65.32 | 256.0 | 229.3 | 26.7 | 0.07345 |
| [1.0, 8] (no underclocking) | 64.99 | 256.0 | 228.5 | 27.5 | 0.07343 |

Identical, and the reason is visible again in the same line: `run 256.0` is exactly `budget_n`.
The per-tick block budget is already the binding constraint, so suppressing a block's budget-tier
eligibility just hands the slot to the next candidate — the total does not move.

**Conclusion: underclocking is currently buying nothing in either material.** It is not free
either — it carries the S2 hazard (a skipped block must still be able to receive mass) and a code
path. Not deleted here, because "inert in two scenes at one budget" is not "inert"; the case to
check before deleting is a scene where `run` sits BELOW `budget_n`, since only there can deferring
a sweep actually remove work. If none exists, delete it.

## Measured 2026-08-20, once the continuous rule actually shipped

`vpar` and settled churn under unquantised rates, the run that had been killed three times.
`diag_overclock_ab oscillation`, grid 512, Square, settle-then-measure:

| overclock | material | churn/cell/tick | vpar |
|---|---|---|---|
| on  | water   | 0.000271 | -0.004 |
| off | water   | 0.000029 | -0.000 |
| on  | drysand | 0.000745 | -0.000 |
| off | drysand | 0.006274 | -0.002 |

At-rest baseline (task #70 fix, HANDOVER §9) is 0.000025 per cell per tick.

**§7b's beat does not appear.** `vpar` stays within ±0.004 of zero in every configuration —
arbitrary, non-nesting rates do not split the period-2 checkerboard parity, which was the specific
theoretical objection to removing quantisation.

**Settled churn is not clean, and this is the open item.** Water churns ~9x the at-rest baseline
with clocking on (0.000271 vs 0.000025); DrySand moves the other way and churns ~8x LESS than
unclocked. Measured under the continuous rule with **no quantised-rule control run**, so it is not
attributable to arbitrary rates specifically — it may be overclocking's, and it may predate this
change. **Run the quantised control before overclocking is ever defaulted on.**
