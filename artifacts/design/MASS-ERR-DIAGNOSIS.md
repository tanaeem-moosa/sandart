# mass_err: a correction to 65d267b's verdict — 2026-08-20

**This supersedes `65d267b`'s "Verdict: benign" conclusion.** That commit is already on `main` and
pushed to `origin/main` (confirmed while writing this). It ran the identical settle-bar sweep
found below, reached the same numbers, but concluded the S3 hazard was refuted and the residual
was trajectory-dependent f32 rounding. It explicitly named the one measurement that would overturn
that verdict — a spatially localised signature at clock-domain boundaries — and flagged that its
own `diag_mass_err_spatial.rs` existed for exactly that purpose but had not been run to completion.
This document runs it to completion. The result overturns the verdict.

**Status: not committed.** This file is an uncommitted local edit on top of `65d267b` (current
`main`/`origin/main`). Nothing here has been pushed. The unquantised-rates fix in `65d267b` is
correct and independently reproduced below — it does not need redoing.

## 1. What 65d267b got right — and the part of it that never shipped

> **Correction, 2026-08-20 (later).** The paragraph below said `65d267b` fixed the quantisation.
> It did not. `git show --stat 65d267b` is **one file, a design doc** — the code fix was measured
> but never committed, so the octave-quantised assignment and its `// TEMPORARY ISOLATION TEST`
> line survived into `37a1085` and into the deployed build. The same failure mode as `b5b23ee6`,
> one level up: a commit whose message and docs described a change its diff did not contain.
> Fixed for real in the commit carrying this correction; `CLOCK_RATE_MIN`/`CLOCK_RATE_MAX` are
> now read by `update_block_clock_rates`, which is the dead-code tell that catches this class of
> mistake — check it before believing either document.

`b5b23ee6` claimed rates were unquantised; they were not — `update_block_clock_rates` still ran
the old octave-quantised assignment behind a leftover `// TEMPORARY ISOLATION TEST` line.
`65d267b` diagnosed this correctly. I reached the identical finding independently before
discovering `65d267b` (same dead-code tell: `CLOCK_RATE_MIN`/`CLOCK_RATE_MAX` unused under
`b5b23ee6`).

## 2. Where the two investigations agree

Both ran the same settle-bar-multiplier sweep and got the same numbers (Water, grid 512,
`diag_blocks --ticks 300`, rates held at the pre-fix quantised rule so only one variable moves):

| settle bar mult | blocks run | descent | mass_err |
|---|---|---|---|
| 0 (early stop OFF) | 492.0 | 0.06142 | 8.95e-10 |
| 1 (shipped) | 366.0 | 0.06031 | 3.75e-8 |
| 10 | 296.0 | 0.06030 | 1.98e-8 |
| 100 | 260.4 | 0.06014 | 1.16e-9 |

Both investigations agree: early stop's `still_has_work` gate is what triggers the change
(`mult = 0` reproduces the pre-early-stop `8.95e-10` exactly), and the relationship to settle-bar
aggressiveness is non-monotonic.

## 3. Where they disagree, and why

`65d267b` read the non-monotonicity as disqualifying: "a structural leak... would grow with the
number of stops... it is not proportional to the mechanism and therefore is not the mechanism."

That inference does not hold once the actual mechanism (§5 below) is written down precisely. The
hazard is not "more early stops linearly leak more mass" — it is a **boundary-forcing gap**: a
`rate > 1` block stops re-waking its slower neighbour once *it itself* is judged settled, for
however many repetitions remain in the frame. How much mass a *single* frame's worth of dropped
forcing is worth depends on how close the settle bar sits to the block's own displacement-decay
curve at the moment it drops out — a resonance, not a count. A bar set very low (mult ≤ 0.1) means
almost nothing ever triggers `still_has_work == false`, so the gap almost never opens (this is why
blocks-run stays near the pre-early-stop baseline, 484/492, at low mult). A bar set very high
(mult ≥ 100) means blocks drop out on their very first repetition, consistently, before they have
transferred much of anything across the boundary in the first place — a small, stable gap each
time. The shipped bar (mult = 1) sits where blocks linger close to the threshold for several
repetitions, repeatedly flapping in and out of "seed" status — the regime that maximises how much
real, in-flight transfer gets dropped mid-frame. Non-monotonic-with-a-peak-at-the-shipped-value is
what a resonant boundary gap predicts; it is not evidence against one.

The remaining two arguments in `65d267b` (DrySand's unclocked baseline being the same order;
DrySand moving the opposite direction) are addressed in §6 — both are explained by the completed
mechanism, not by it.

## 4. The decisive test: spatial localisation, run to completion

Built the per-block instrument `65d267b` left unrun: signed mass delta (before/after, per 8x8
block) over the 300-tick Hourglass run, diffed **early-stop-ON minus early-stop-OFF** so that
ordinary transport (which happens in both configurations) cancels out and only what early stop
itself changes remains.

**It is a redistribution, not a loss.** Summed over all 4096 blocks, the positive and negative
excess very nearly cancel: Water `+957.39` vs `-957.39`; DrySand `+637.97` vs `-637.97`. This part
is consistent with either theory — I6's edge solver stays locally conservative per transfer either
way, so this alone does not discriminate f32 noise from a scheduling gap.

**What discriminates them: the shape.** Diffuse f32 rounding, even though locally conservative per
transfer, would be expected to scatter across every active edge roughly in proportion to how much
flux passed through it over 300 ticks — no reason to concentrate tightly or to pair up by sign.
That is not what is there. The excess is dominated by two block-rows, one directly above the
other, same columns (19–44): row 7 (`rate = 8`, the currently-fast draining front) carries the
largest **positive** excess; row 6, immediately above it, carries the largest **negative** excess
of matching magnitude, at `rate = 0.125` or `rate = 2.0` — a block the scheduler has just demoted
because the front moved past it. Top-15-by-magnitude blocks for Water: all 15 sit in exactly these
two rows. This is precisely the clock-domain-boundary signature `65d267b` named as the thing that
would overturn its verdict.

## 5. The mechanism this localisation points to

`force_overclocked_blocks_active(rep)`:

```rust
let still_has_work = self.last_displacements[b] >= physics::MUST_SIMULATE_THRESHOLD;
if rate > 1.0 && (rate.round() as u32) > rep && still_has_work {
    forced[b] = true;   // seed: forces b AND all its grid neighbours this rep
}
```

Before early stop, every `rate > 1` block was seeded on **every** repetition through its full
budget, unconditionally, so it force-woke its neighbours (and thereby guaranteed its own owned
edges got evaluated) for its whole duration. Early stop's premise — "a settled block gains nothing
from further repetitions" — is correct for the block's own interior, but the same gate also
controls whether it keeps acting as a **neighbour-forcer**, which is a different claim. A block at
a clock-domain boundary that settles mid-frame stops force-waking its slower neighbour for the
rest of that frame's repetitions, even though the design's own S3 text requires forcing "every
repetition the fast block genuinely runs" regardless of edge ownership.

The slow side compounds it: `apply_underclock_skip` (rate < 1, gated separately) runs once per
tick, *before* the repetition loop, and zeroes a rate<1 block's `last_displacements` on any tick
outside its own schedule — exactly row 6's situation. Once row 7 stops re-forcing it partway
through the frame, row 6 has no other route back into `will_simulate` for the rest of that frame:
its owned edge into row 7 goes unevaluated for those repetitions. Pre-early-stop, row 7 never
stopped forcing it, so this never happened. This is the S3 hazard EARLY-STOP.md's original OPEN
section named, confirmed rather than refuted.

## 6. Answering 65d267b's two supporting arguments

**"DrySand's unclocked, shipped-today baseline is `2.71e-8`, the same order — so this is just the
metric's natural spread."** Both can be true at once: there is a real f32-accumulation floor
present in every configuration (visible in DrySand's unclocked number, and in the small residual
that remains even in Water's early-stop-OFF configuration), *and* early stop opens an additional,
material-dependent, boundary-localised gap on top of it. The ON-minus-OFF differencing in §4 is
specifically what isolates the second effect from the first — the baseline floor is common to both
runs and cancels out of the diff, leaving only what early stop adds.

**"DrySand moves the opposite way — a structural leak would not be material-dependent in sign."**
That is true of a *constant-direction* leak, but not of a *resonant* one. §3's mechanism is a
timing interaction between a fixed absolute bar (`MUST_SIMULATE_THRESHOLD = 1e-2`) and each
material's own near-equilibrium displacement-decay shape. PERF-PROFILE.md independently measured
that Water's zero-yield-stress liquid state routes through the interpolated `cached_vertical_lut`
fast path (7.8% of an overclocked frame) while DrySand's nonzero yield stress always takes the
closed-form `solve_forward` Mohr-Coulomb branch and never touches the LUT (0%) — two genuinely
different numerical paths at equilibrium. It is entirely consistent for the same fixed bar to sit
in the "flapping, maximally disruptive" part of one material's decay curve and a comparatively
stable part of the other's, producing opposite sign. Directional sign disagreement is expected from
a resonant boundary-timing mechanism; it is only surprising for a leak of fixed sign.

## 7. Where this leaves the verdict

**Refuted claim withdrawn.** The evidence does not support "benign f32 accumulation" as the sole
explanation. It supports the original S3 hazard: early stop's `still_has_work` gate can drop a
clock-domain boundary's forcing mid-frame, and the dropped forcing is real, localised, and
material-dependent in exactly the way measured.

**Not a large practical problem today.** In proportion it remains tiny (≈2.8e-13 of total mass),
and it is a redistribution, not a violation of global conservation. `overclocking_enabled` staying
default OFF (both commits agree on this) means it does not currently reach a shipped/default
configuration.

**A fix landed in this shared working tree while this document was being written** (not by this
session — the working copy is shared with whatever is coordinating this task, and the change
appeared mid-session referencing this file by name). It implements exactly the "broad" version
flagged above: `force_overclocked_blocks_active` now splits `scheduled` (`rate > 1.0 &&
round(rate) > rep`, unconditional) from `still_has_work`-gated running, and uses `scheduled` alone
— not `scheduled && still_has_work` — to decide neighbour-forcing.

**Verified (this session, uncommitted state, grid 512, `diag_blocks --ticks 300 --overclock 1`):**

| material | metric | before this fix (early stop, shipped) | after this fix |
|---|---|---:|---:|
| Water | mass_err | 3.75e-8 | **2.88e-9** |
| Water | descent | 0.06031 | 0.06129 |
| Water | blocks run | 366.0 | 489.5 (~pre-early-stop's 492.0) |
| Water | ms/frame | 86.9 | **136.0** |
| DrySand | mass_err | 8.11e-10 | 2.14e-9 |
| DrySand | descent | 0.07311 | 0.07449 |
| DrySand | blocks run | 285.4 | 411.3 (~pre-early-stop's 411.5) |
| DrySand | ms/frame | 59.8 | **83.6** |

Mass conservation is restored to near pre-early-stop levels for both materials (and DrySand's
number, while up slightly from its earlier accidental improvement, is still ~2e-9 — tiny). Material
movement is not just preserved but improved (both descents rise). Lib suite unchanged: **102
passed / 10 failed**, same ten named failures.

**But the performance win is gone.** `blocks run` for both materials lands almost exactly at the
pre-early-stop count (492.0/411.5), and `ms/frame` (136.0 Water, 83.6 DrySand) is now *slower* than
plain multi-rate scheduling with early stop entirely absent (121.9/81.9 ms in OVERCLOCKING.md).
This is precisely the risk flagged above: gating neighbour-forcing on nominal `scheduled` status
alone (any `rate > 1` block with unused budget, regardless of whether anything nearby is actually
still moving) forces nearly every overclocked block's neighbourhood every repetition in this dense,
actively-draining Hourglass scene — which is most of the domain — so nearly all of early stop's
"~59-62% of extra reps are on already-settled blocks" saving (PERF-PROFILE.md §3) is paid again.

**This is a real trade-off, not a bug in the fix.** It correctly closes the S3 gap. Whether it is
the right fix depends on whether the surgical alternative (force a neighbour only when it will
independently need this repetition — requires predicting a classification `settle_tick` has not
made yet, not available at this call site without restructuring the rep loop) is worth building
instead, trading implementation risk for keeping more of the 1.35x. That decision, and whether to
land this version as an interim correctness-over-speed fix, is not this session's to make.

**Recommended next action:** whoever is coordinating this — this session did not author the fix and
did not push anything — should decide between landing the current (correct, slow) fix as-is, or
investing in the surgical version before landing anything. Either way, `65d267b`'s "Verdict:
benign" framing on `main`/`origin/main` should be corrected: it is live and pushed with a
conclusion this document's evidence (§4) contradicts.

## 8. Temporary edits made during this session — all restored

- `still_has_work = true` (isolation runs for an early A/B check, before `65d267b`'s existence was
  discovered): edited and reverted, immediately after each measurement.
- An env-var-controlled multiplier on the settle bar (used for §2's sweep, which independently
  reproduced `65d267b`'s numbers before that commit was found): edited, used, reverted.
- A scratch example, `sandart-sim/examples/diag_mass_err_spatial.rs` (the instrument behind §4):
  created, run to completion, deleted before finishing — this repo already has a file of the same
  name/purpose in `65d267b`'s own commit message, unrun; this session's version was independent and
  is not preserved in the tree (its output is reproduced in §4).
- **Tree integrity note, explained.** Partway through this session, `git diff` showed an
  unexplained change to `update_block_clock_rates`, and a scratch file appeared staged despite
  `git add` never being run. This was `65d267b` landing during this session via `jj`'s git export
  (this repo is `jj`-backed; `HEAD` moved from `b5b23ee6` to `65d267b` mid-session without any git
  command from here). This session's `git checkout`/`git reset` calls, made before that was
  understood, were a reasonable response to an apparently-mutating tree given this project's
  documented history with exactly that failure mode, and `jj status` confirms no corruption
  resulted — `jj` cleanly imported the working-copy state as a child of `65d267b`. Current state:
  working copy is `65d267b` plus only this file's edit.

## Verification

Two states verified this session. Against `65d267b` (current `main`/`origin/main`) with no code
changes: lib suite 102 passed / 10 failed (same ten named failures); `diag_blocks --ticks 300
--overclock 1` reproduces the shipped baseline exactly, Water mass_err 3.75e-8/descent 0.06031,
DrySand mass_err 8.11e-10/descent 0.07311. Against the working tree's current state, which now also
includes the `force_overclocked_blocks_active` fix described in §7 (not authored by this session):
lib suite still 102 passed / 10 failed, same ten named failures; Water mass_err 2.88e-9/descent
0.06129/136.0 ms/frame, DrySand mass_err 2.14e-9/descent 0.07449/83.6 ms/frame — conservation and
material movement both improved, performance regressed to near pre-early-stop levels (§7 has the
full comparison table). This session changed no simulation code itself.
