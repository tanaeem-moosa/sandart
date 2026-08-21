# The coarse-grid flow correction — 2026-08-20 (night)

The user's question, which is what this is: *"how about ... using the coarse flow to move more
material than would be safe otherwise? (right now lateral flow is bound by half the difference. but
we know some flow information from coarse sim)"* — and, on scope and safety: *"should we make it
specific to lateral flow or all flow? also let's have dampening param. coarse is by definition
approximation."*

## 1. Why the fine level cannot fix this by running harder

DrySand's lateral edges move `c_sq * damping ≈ 0.08 * 0.76 ≈ 6%` of a height difference per tick;
Water's move ~23% (`wave_params`, `physics.rs`). `PRESSURE_RATE_FULL_AT_ROWS_OF_HEAD`'s own doc
comment already records that these "sit at the CFL bound", so they cannot simply be raised.

**That bound is a local explicit-scheme stability limit.** It governs how fast *local relaxation*
may propagate information. It is not a statement about how much mass may legally move — conservation
is that statement, and the FCT limiter already enforces it exactly.

And local relaxation is a **smoother**. It kills short-wavelength error quickly and long-wavelength
error at a rate that degrades as the wavelength grows. Spreading a pile sideways across many blocks
is precisely the long-wavelength mode. This is the prediction that reframed the session:

> Running more lateral sub-steps will polish the local repose angle and barely move the pile.

which is consistent with everything the previous session measured — extra repetitions bought frame
time, not spread, and "no block reaches 8x, nothing is wide enough to ramp" (SESSION-HANDOVER
2026-08-20 evening §3). The mechanism was right and it was attached to the wrong error mode.

The coarse level is where the smooth mode is solvable. That is the entire reason multigrid exists,
and FLOW-DIRECTION.md had already measured the gap: the coarse level's flow is **2.2x more lateral**
than the fine level's on DrySand, 1.5x on Water.

## 2. The formulation

A coarse-grid correction, applied as a **defect**, per block face:

```
defect[face] = damping * ( coarse_flux[face] * t*t  -  fine_flux[face] )
```

- `coarse_flux[face]` — the signed flux the coarse sim **actually performed** across that tile face
  this tick. Not a reconciling flux, not a solve: the coarse level is a real `NestedSim` running the
  shipped `settle_tick`, so it already computed this. Read it, do not re-derive it.
- `* t*t` — the coarse level holds a tile's height as an AVERAGE over its `t*t` fine cells, so one
  unit of coarse height is `t*t` (= 64 at shipped geometry) units of fine mass.
- `fine_flux[face]` — the signed mass the fine level actually moved across the same physical face,
  summed over every repetition of the frame.

Applied after the frame's sub-steps have all run, distributed over the face's `block_size` fine
edges proportional to each edge's availability, through the existing limiter.

**This retires the blocker the previous handover recorded.** §5 item 2 there listed lateral-only
sub-steps as blocked on a directional signal costing ~1,220 SOR sweeps per tick and needing
multigrid V-cycles. That blocker existed only because the search was for the *reconciling* flux
(solve `lap(phi) = delta`, take `grad(phi)`). The flux the coarse level already performed is a
different quantity, it is the one the defect form needs, and it costs an accumulator.

### Why it is a correction and not a fudge

1. **It is a flux.** Every transfer is `data[src] -= x; data[dst] += x`. Mass is conserved exactly,
   by construction, in divergence form. Asserted in `coarse_flow_correction_conserves_mass_on_both_axes`.
2. **It is a defect.** If the fine level already moved as much as the coarse level did, the
   correction is identically zero. No double counting.
3. **The existing limiter is the safety net.** Per edge the transfer cannot exceed the donor's height
   or the acceptor's headroom, so nothing goes negative or past capacity no matter what the coarse
   level asks for.

Deliberately **not** clamped to the local half-difference: exceeding what local relaxation could have
moved is the entire point. Conservation and the limiter are what keep it safe, not the stability
bound.

### The ledger, and the trap in it

Both levels' realised flux is recorded at `flux_edge_apply`, signed and per edge, selected by level
the same way `flux_dir_set_coarse` selects a diagnostic bin.

**`flux_edge_apply` is not the only transport path.** Sand's angle of repose lives entirely in the
granular CA's lateral flow (`try_move`), not in the flux solver. A ledger watching only the flux
solver would have read DrySand's realised lateral transport as near zero and then "corrected" a
deficit that did not exist — a large, plausible-looking, entirely wrong result. `try_move` is
recorded too (`lat_ledger_record_ca`), with diagonal CA moves decomposed as a staircase.

## 3. Measured — `diag_lateral_corr`, grid 512, hourglass, 300 ticks, overclocking off

`spread` is the mass-weighted std-dev of x over the bottom quarter, the same definition
`diag_blocks` uses, so these are directly comparable with the previous session's numbers.

### DrySand — the material the repose problem lives in

| damping | spread | vs off | frame ms | block-steps | delivered | limited | descent |
|---|---|---|---|---|---|---|---|
| off | 5.91 | base | 13.69 | 417 | — | — | +0.03653 |
| 0.05 | 6.43 | **+8.7%** | 12.16 (−11%) | 506 (+21%) | 98.5% | 0.9% | +0.03082 |
| 0.25 | 7.72 | **+30.6%** | 14.47 (+6%) | 578 (+39%) | 92.1% | 3.4% | +0.03241 |
| 0.40 | 8.34 | **+41.0%** | 15.08 (+10%) | 598 (+43%) | 88.8% | 4.7% | +0.03368 |
| 0.60 | 8.93 | **+51.0%** | 16.07 (+17%) | 612 (+47%) | 81.6% | 9.3% | +0.03566 |
| 1.00 | 9.09 | **+53.8%** | 19.22 (+40%) | 697 (+67%) | 78.4% | 14.6% | +0.04601 |

Spread rises monotonically and saturates around 0.6–0.8. Descent is unharmed through 0.6 (0.03653
off vs 0.03566) and only rises at 1.0. `mass_err` is ~1e-9 throughout — **better** than the
uncorrected baseline's 2.71e-8.

**For comparison: coarse pressure coupling costs ~36% frame time for ~7% spread.** This is +41%
spread for +10% at damping 0.4.

### Water

| damping | spread | vs off | frame ms | block-steps | delivered | limited | descent |
|---|---|---|---|---|---|---|---|
| off | 8.77 | base | 16.80 | 407 | — | — | +0.00738 |
| 0.05 | 9.88 | +12.8% | 27.21 (+62%) | 765 (+88%) | 96.0% | 5.2% | +0.00836 |
| 0.25 | 12.01 | +37.0% | 31.78 (+89%) | 951 (+134%) | 94.8% | 7.4% | +0.00973 |
| 0.40 | 13.07 | +49.0% | 32.72 (+95%) | 972 (+139%) | 95.3% | 8.0% | +0.01240 |
| 0.60 | 14.26 | +62.7% | 31.53 (+88%) | 929 (+128%) | **74.7%** | **69.2%** | +0.03795 |
| 1.00 | 15.43 | +76.0% | 31.30 (+86%) | 916 (+125%) | **45.2%** | **91.7%** | +0.03732 |

**There is a regime change between damping 0.4 and 0.6 on Water.** Requested transport jumps 285 →
5213 per tick (18x for a 1.5x change in damping), the limited fraction goes 8% → 69%, and descent
triples. That is the predicted feedback loop: correction drives the levels further apart, which
demands more correction. **Water should stay at or below 0.4.** DrySand shows no such cliff.

### The cost is real physics, not overhead

Frame time and executed block-steps rise together (Water at 0.25: +89% ms, +134% steps), which is
the cost model from SESSION-HANDOVER §2 holding — and it is sub-linear in the same way that section
records. The correction costs what it costs because it **un-settles material the scheduler had
written off**, and that material then runs. That is the correction working, not overhead.

One real bug was found this way and fixed. The first implementation bumped both blocks' displacement
to `MUST_SIMULATE_THRESHOLD` on any correction at all. Since the NUMBER of faces carrying some
correction is nearly independent of damping (844 at 0.05, 796 at 1.0 on Water), that produced a
large *damping-independent* frame-time cost — +93% on Water even where the correction moved almost no
mass. The hint is now scaled by the mass actually moved, exactly as `activate_neighbor` does on the
ordinary transport path. DrySand's cost at damping 0.05 went from +5% to −11%.

## 4. Lateral or all flow — the measurement, and it went against the prior

The formulation is direction-agnostic; the axis is selectable (`CorrectionAxes::{Lateral, Vertical,
Both}`). My prior was that `Lateral` was right — the measured deficit is lateral, downward transport
is already at its one-cell-per-tick ceiling, and the coarse level models neither repose nor free fall
so a vertical correction would fight the two fine-level mechanisms that work.

**The measurement disagrees. `Both` wins on spread, on both materials.** 300 ticks, grid 512:

| material | axes | damping | spread vs off | descent | frame ms vs off | limited |
|---|---|---|---|---|---|---|
| DrySand | off | — | base (5.91) | +0.03653 | base (15.69) | — |
| DrySand | lateral | 0.50 | **+46.7%** | +0.03459 | −2% | 6.7% |
| DrySand | vertical | 0.50 | +12.4% | +0.04301 | −1% | 22.2% |
| DrySand | **both** | 0.50 | **+68.5%** | +0.04462 | +6% | 16.2% |
| Water | off | — | base (8.77) | +0.00738 | base (16.62) | — |
| Water | lateral | 0.25 | +37.0% | +0.00973 | +90% | 7.4% |
| Water | vertical | 0.25 | +7.3% | +0.01817 | +88% | 12.0% |
| Water | **both** | 0.25 | **+48.8%** | +0.01756 | +100% | 12.4% |

Vertical alone buys little spread (+12% / +7%) but substantial DESCENT (+18% on DrySand, +146% on
Water). Combined, the two axes add: `Both` at damping 0.5 on DrySand gives +68.5% spread *and* +22%
descent for +6% frame time. Only ~20% of `Both`'s applied mass is lateral, yet it out-spreads the
lateral-only run — so the lateral gain is partly *indirect*: correcting the vertical deficit drains
the neck faster, which builds the lower pile faster, which gives the lateral correction more pile to
work on.

**The shipped default is still `Lateral`, deliberately.** `Both` measured better on the metric, but
it also changes descent by ~22% on DrySand, which is a change to how the hourglass *looks and times*
rather than to how far material spreads. That is a judgement about the piece, not about the solver,
so it is a dropdown rather than a silent default. The evidence says pick `Both`; the call is the
user's.

## 5. Damping

`COARSE_CORRECTION_DEFAULT_DAMPING = 0.5`, **and it should now be changed** — see §7. Under-relaxing
a coarse-grid correction is standard whenever the coarse operator is not a Galerkin projection of
the fine one, which this one certainly is not: different grid, and no model of the angle of repose
whatsoever. The user's framing was the correct one — *"coarse is by definition approximation."*

## 6. What shipped

- `physics::apply_coarse_flow_correction` + the flow ledger (`lat_ledger_*`), recorded at both
  `flux_edge_apply` and `try_move`.
- `DrawingSimulation::{coarse_flow_correction, coarse_correction_axes, coarse_correction_damping,
  last_frame_correction}`. **Default OFF**, like every other debug toggle in that group.
- Debug panel: a checkbox, a "Correction strength" slider, a "Correct across" selector, and a live
  readout of asked/moved/faces/limited. The slider and selector are hidden while the toggle is off.
- `diag_lateral_corr` — sweeps `--damping` and `--axes`, reports spread, descent, `mass_err`, frame
  time, block-steps and the correction's own accounting.
- `coarse_flow_correction_toggle.rs` — four tests: off-by-default is bit-identical, on diverges,
  `damping = 0.0` is bit-identical to off, and **mass is conserved on all three axis modes at
  damping 1.0**.

## 7. Next

1. **Set the shipped damping and axis from the sweeps, not from the starting guesses.** Damping:
   ~0.4–0.6 for DrySand, ≤0.4 for Water (the Water cliff, §3); a single default serving both is
   ~0.4. Axis: `Both` measured better on both materials (§4). Both are **the user's call**, and both
   are one edit from `COARSE_CORRECTION_DEFAULT_DAMPING` / the sim's default axis.
2. **Watch for repose ringing at the flanks.** The predicted failure — coarse asks to flatten a pile
   below its angle, fine restores it, the flanks oscillate — has not been looked for on screen, only
   bounded by the descent and mass numbers. This needs eyes on the deployment, and it is the reason
   damping is exposed rather than baked in. If it appears, the honest fix is a clamp that stops the
   correction driving the local slope below repose — physical, not a fudge.
3. **The Water regime change above 0.4 deserves a diagnosis, not just a limit.** The 18x jump in
   requested transport for a 1.5x damping change is a feedback loop with a threshold, and knowing
   what sets the threshold is worth more than avoiding it.
4. **Selective coupling is now cheap to try.** SESSION-HANDOVER §5 item 4 wanted "the local
   disagreement is predominantly lateral" as a per-block quantity. The ledger is exactly that
   quantity, already computed.
