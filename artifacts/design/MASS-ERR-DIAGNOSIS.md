# mass_err diagnosis, and a correction to what actually shipped — 2026-08-20

## 1. The correction, first, because it invalidates part of the previous commit

**`b5b23ee6` claimed rates were unquantised. They were not.** The commit message asserted the
power-of-two quantisation was replaced by a continuous rule; the doc comments said so too. The
arithmetic did not: `update_block_clock_rates` still contained the octave-stepped assignment behind
a line reading `// TEMPORARY ISOLATION TEST: restore old octave-quantised assignment for a "before"
vpar run` — left by an agent that was killed before restoring it, and committed by the main thread,
which verified the doc comments rather than the code path.

So the user's explicit request ("without quantizing at power of 2") was **not** in the deployed
build. It is now. The lesson is narrow and worth keeping: **when a commit claims a behaviour
changed, read the code that implements it, not the comment that describes it.** Doc comments in this
tree are written before the code they describe as often as after.

The continuous rule now in place is `rate = clamp(signal / CLOCK_DELTA_REF_FRAC, 1/8, 8)`, with the
octave-stepped hysteresis gone too — early stop makes the precise rate value much less important,
since `rate` is only an upper bound on repetitions and a settled block stops consuming them anyway.
A rate that is somewhat too high now costs nothing, which is what the hysteresis existed to prevent.

## 2. The mass_err question: early stop is the trigger, but it is NOT a structural leak

The suspicion was that early stop leaves transfers half-applied — a block stopping while it still
owns edges a running neighbour needs (§7b's S3 hazard). **That hypothesis is refuted by a sweep.**

Isolating early stop alone (rates held at the old quantised rule, so only one variable moves), grid
512, Water, `diag_blocks --ticks 300`, sweeping a multiplier on the settle bar:

| settle bar mult | blocks run | descent | mass_err |
|---|---|---|---|
| 0 (early stop OFF) | 492.0 | 0.06142 | 8.95e-10 |
| 1 (shipped) | 366.0 | 0.06031 | **3.75e-8** |
| 10 | 296.0 | 0.06030 | 1.98e-8 |
| 100 | 260.4 | 0.06014 | 1.16e-9 |

**The error is non-monotonic in how often early stop fires.** A structural leak — mass lost per
early stop at a clock-domain boundary — would grow with the number of stops. Instead the error peaks
at the shipped setting and falls back to baseline as stopping becomes *more* aggressive, while
blocks-run falls monotonically (492 → 366 → 296 → 260). Whatever this is, it is not proportional to
the mechanism, so it is not the mechanism.

`mult = 0` reproduces `8.95e-10` exactly — the pre-early-stop value — confirming the sweep is a
clean isolation.

**What it looks like instead: trajectory-dependent floating-point accumulation.** Every transfer does
`h_a -= d; h_b += d` in f32, and each side rounds independently, so conservation is exact only up to
rounding. Changing the schedule changes the trajectory, which changes where those roundings land and
how they cancel. Supporting evidence:

- **The metric's natural spread across shipped configurations already covers this range.** DrySand
  *unclocked and shipped today* measures `2.71e-8` — the same order as the "regression". Water
  unclocked is `2.20e-9`. A 12x spread between two shipped configurations, with no clocking
  involved at all.
- **DrySand moves the OPPOSITE way** under the same change (`5.78e-9 → 8.11e-10`, and `1.47e-9` with
  unquantised rates). A structural leak would not be material-dependent in sign.
- In proportion the number is ~2.8e-13 of the ~135,648 total mass.

**Verdict: benign, and demonstrated rather than asserted.** No tolerance was tuned, no fixup or
renormalisation pass was added — I6's "no reconciliation, no fixups" stands untouched.

**What would change this verdict:** a spatially localised signature. If the discrepancy were shown
to accumulate at clock-domain boundaries specifically, that would outrank the argument above.
`sandart-sim/examples/diag_mass_err_spatial.rs` exists for exactly that measurement and was not run
to completion. Worth doing before overclocking is ever defaulted ON.

## 3. Measurements with rates genuinely unquantised

Grid 512, `diag_blocks --ticks 300`, overclocking ON:

| material | ms/frame | descent | blocks run | mass_err |
|---|---|---|---|---|
| Water | **80.56** | 0.06023 | 297.5 | 3.07e-8 |
| DrySand | **54.70** | 0.07313 | 256.0 | 1.47e-9 |

Against 121.9 ms (Water) and 81.9 ms (DrySand) with clocking on and early stop off: **1.51x and
1.50x**, better than the 1.35x reported for the quantised build, with material movement preserved
(descent within 2%).

Cumulative, Water at grid 512: 121.9 ms → 80.6 ms, while descent per tick stays ~8x the unclocked
0.00738.

## 4. Still not measured

`vpar` and settled churn under unquantised rates — §7b's residual concern that arbitrary rates beat
against the known period-2 checkerboard mode. `diag_task70_rest_color_mixing_and_checkerboard`'s
`vpar` column is the direct read. Three separate agents have now been killed mid-run attempting it.
**This should be settled before overclocking is defaulted on**, alongside the spatial mass check.
