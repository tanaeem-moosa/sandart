# The vessel masks were never left-right symmetric

2026-09-08, against `b28f127`. Reported symptom: water in an hourglass is visibly asymmetric,
much worse with three necks, noticeable at 256 but not 128.

## 1. The cause: a half-cell mirror axis

`eval_sandbox_shape` computed `center_x = w as f32 / 2.0`. Cell centres are the integer indices
`0..=w-1`, so the grid's true mirror axis is `(w-1)/2`. With `w/2` the mirror pair `(x, w-1-x)`
gives `dx = x - w/2` and `w/2 - 1 - x` — not negatives, off by exactly one cell. **Every vessel in
the app was evaluated half a cell left of centre and no shape was left-right symmetric.**

Mirror mismatches before the fix, and zero for all nine after:

    shape                  grid 128   grid 256   grid 512
    Hourglass                  406        816       1632
    MultiNeckHourglass         626       1244       2480
    MultiStageHourglass       1584       3220       6460

The exact 2x per doubling is the signature of a one-cell boundary offset. MultiNeck carries ~50%
more than plain Hourglass, which is the reported "much worse with 3 neck".

`test_vessel_masks_are_left_right_symmetric` pins this.

## 2. What the fix actually bought

Water draining an hourglass, upper chamber only, full budget, ticks scaled with resolution
(`diag_water_hourglass_mirror_asymmetry`):

                        worst mirror        final SIGNED lean
                        old  ->  new        old      ->  new
    Hourglass  128    0.6647 -> 0.6353    -0.0130   ->  -0.0000
    MultiNeck  128    0.7180 -> 0.7134    -0.0093   ->  +0.0000
    Hourglass  256    0.6942 -> 0.6816    -0.0065   ->  -0.0000
    MultiNeck  256    0.7388 -> 0.7299    -0.0045   ->  +0.0001

The fix eliminates the PERSISTENT SETTLED LEAN — under the old axis every run settled biased the
same direction (all four negative, ~1% of total mass); after, it is zero. That is the historical
"tendrils usually on the left" complaint.

It barely touches the transient (1-4%). **The large mid-drain asymmetry, mirror error 0.64-0.73, is
untouched and is the remaining defect.**

## 3. "Worse at 256 than 128" is not a resolution-dependent asymmetry

First measurement appeared to confirm it: at equal tick counts, 256 ended at mirror 0.0318 while
128 ended at 0.0000. That was an artifact. Transport is clamped to one cell per tick and a 256 cell
is half the physical size, so the same physical drain needs twice the ticks; equal tick counts
compare a finished drain against an unfinished one. **With ticks scaled to resolution, 256
converges to ~0 as well** (0.0000 / 0.0002).

What differs is DURATION, not asymmetry. At 256 the drain takes twice as long, so the transient —
where the asymmetry lives — is on screen far longer. This is the +-1.0 clamp being felt again, not
a second bug.

## 4. Red-black edge colouring: the prior experiment, re-tested

`d6d843b` (2026-07-31, off main) implemented red-black edge colouring on the lateral pass. It
worked: fully tick-phase invariant, all eight mechanisms in `test_tick_phase_mechanism_isolation`
bit-identical to baseline, even/odd runs bit-identical, magnitude roughly halved (worst
1.109e-1 -> 5.277e-2). But `late_persistent_run` stayed at 75, and it concluded the residual lean is
ORDER-INDEPENDENT and unreachable by any scan-order fix. Held off main for regressing
`test_settled_liquid_sleeps_and_wakes` (pouring_slept 54.7% -> 75.4%) and ~8% perf.

That conclusion was drawn against a mirror map of `x -> w - x` — the buggy axis. It was worth
re-testing rather than citing, because an order-independent lean is exactly what a fixed geometric
offset produces.

**Re-tested with the corrected axis and corrected metric: the conclusion holds.**
`late_persistent_run` is 75 (even) / 43 (odd), essentially unmoved from 75 / 45. The residual bias
is order-independent AND geometry-independent. Whatever it is, it is in the physics expressions
themselves, not the traversal and not the mask.

## 5. What this test was doing wrong, and the pattern to watch for

`test_water_blob_stays_left_right_symmetric_under_gravity` had DOCUMENTED THE BUG AS A FACT: it
measured the mask, correctly found it symmetric about `w - x` rather than `w - 1 - x`, and adapted
its mirror map and blob placement to the buggy axis instead of reporting it. Every number the
asymmetry work has quoted since was measured against that axis.

`test_no_floating_sand_under_gravity` had the same disease in a different form: it reimplemented
the hourglass boundary INLINE, TWICE (once to fill, once to assert), with a hardcoded
`center_x = 32.0`. That copy never matched `eval_sandbox_shape` exactly; the axis correction
widened the gap to a full cell and it began reporting "floating sand" — sand its own fill had
placed in columns the vessel does not support. It now fills and asserts from the real mask. **This
was never reachable in the app**, where fill and mask come from the same evaluator.

If a test carries its own copy of the shape math, that is the bug. Fill from the mask.

## 6. Fallout still open

`test_cascade_no_dam_or_neck_merge_across_chamber_count_range` fails, and only the MultiStage /
cascade shape family is affected. Both of that family's clamps were tuned against the integer axis:

- `multistage_neck_half_width`'s floor was 0.5. On a half-integer axis a cell's `|dx|` is always
  0.5, 1.5, 2.5..., so `|dx| < 0.5` admits NO cell and the neck closed completely, trapping
  everything above it. **Fixed: floor raised to 1.0**, which opens the two central cells — the
  narrowest neck symmetric about the axis. Necks on this axis are necessarily even-width.
- `anti_merge_ceiling = (chamber_w / 2.0 - 0.5).max(0.5)` has the same sensitivity and has NOT been
  re-derived. At `w=64, chambers=11, neck_width=0.06` the wall between adjacent necks opens and
  chambers merge.

Hourglass and MultiNeckHourglass — the shapes the reported symptom is in — are unaffected.

## 7. Next

The remaining defect is the mid-drain transient, mirror error 0.64-0.73, worse with more necks. It
is not geometry and not scan order; both are now ruled out by measurement rather than argument.
`diag_water_hourglass_mirror_asymmetry` is the metric to judge any candidate against — it is the
reported scenario, unlike the water blob, whose `late_persistent_run` red-black already failed to
move twice.
