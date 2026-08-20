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

## Not measured

`vpar` and settled churn under unquantised rates — the agent was killed during that run. §7b's
residual concern about arbitrary rates is a beat against the known period-2 checkerboard mode, and
`diag_task70_rest_color_mixing_and_checkerboard`'s `vpar` column is the direct read. **Worth doing
before overclocking is ever defaulted on.**
