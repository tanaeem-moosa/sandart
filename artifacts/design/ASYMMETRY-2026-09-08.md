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

## 8. Red-black retried, and the trade it revealed

Retried on 2026-09-08 at the user's request, against current `main` rather than July's solver.

**It was NOT subsumed by capacity arbitration**, contrary to `d6d843b`'s closing note. The
shared-endpoint write is UPSTREAM of arbitration: the lateral flux call writes `cell_avail` and
`cell_freecap` for both endpoints, consecutive lateral edges along a row share an endpoint (edge
(x, x+1) and edge (x+1, x+2) both write cell x+1), last writer wins, and which is last is the sweep
direction. `accumulate_edge_totals` then reads those arrays as it goes, so the oversubscription
bookkeeping arbitration depends on was itself direction-dependent.

Structural differences from July: the lateral edge now carries the GRANULAR share too (Stage C), it
writes candidates rather than applying flux, and arbitration runs per-phase inside the phase loop.
So the pass sits at the end of phase 1's traversal, before pressure projection — not after the
phase loop as in `d6d843b`.

### What it bought

`test_tick_phase_mechanism_isolation`, before -> after:

    mechanism                        worst                  final              late_run
    REFERENCE (all 0)          5.6845e-2 -> 2.6940e-2   8.6467e-3 -> 1.6880e-2   44 -> 40
    Block-level x order        4.6161e-2 -> no-op       1.1543e-2 -> no-op       43 -> 40
    Cell-level lateral sweep   6.4742e-2 -> no-op       2.2851e-2 -> no-op       42 -> 40
    RNG seed                   7.5810e-2 -> 3.9442e-2   2.0887e-3 -> 1.5517e-2   45 -> 42

SEVEN of eight mechanisms are now bit-identical to baseline; only the RNG still moves anything.
Baseline `worst` fell 53%.

Note the isolation harness's own documentation was STALE before this: it claimed only two
mechanisms move anything and that "the CA checkerboard and RNG seed live in the granular path a
liquid scenario never reaches". The RNG demonstrably moves it. That is not the dispersion term —
`dispersion` scales by `tau`, which scales by `granular_share = 1 - cell_liquidity`, so it is
identically zero for water; setting `DISPERSION_TAU_FRAC = 0` leaves the drain metric bit-identical.
Whatever RNG path reaches liquid, it is not that one, and it is unidentified.

On the reported scenario (`diag_water_hourglass_mirror_asymmetry`):

    grid/shape        worst mirror         worst signed
    Hourglass 128   0.6353 -> 0.4661    +0.0652 -> -0.0148
    MultiNeck 128   0.7134 -> 0.4546    -0.0304 -> +0.0086
    Hourglass 256   0.6816 -> 0.4426    +0.0658 -> +0.0073
    MultiNeck 256   0.7299 -> 0.5053    +0.0194 -> -0.0054

MultiNeck stops being worse than single-neck at 128 — the "much worse with 3 necks" signature goes.
July's blocker, `test_settled_liquid_sleeps_and_wakes`, does NOT recur.

### What it cost, and the actual lever

`test_liquid_flowing_liquid_does_not_stand_in_walls`, enclosed void cells:

                  voids@120   voids@160    total
    baseline          51          2         9509
    red-black        167         77        20658
    thresholds      <=150       <=20      <=34000

Draining liquid clings to walls ~3x longer.

**The diagnosis "red-black loses the sideways cascade a directional sweep provided, so transport
slows" is WRONG, and was refuted directly.** Running two colour sweeps per tick (`[0,1,0,1]`) to
restore transport made symmetry dramatically BETTER — worst mirror 0.1522 / 0.1761 / 0.1016 /
0.1354, a 4-7x improvement on the untouched 0.64-0.73 — and voids WORSE still (238 / 159 / 32465).

So symmetry and drainage move in opposite directions on one dial, and the dial is HOW OFTEN THE
LATERAL EDGE IS INTEGRATED RELATIVE TO THE GRAVITY-ALIGNED ONE. This is an operator-split balance
problem, not a colouring problem. Red-black moves the dial one notch; two sweeps move it two.

The next thing to try is therefore NOT a different colouring or a point on this curve, but giving
the gravity-aligned edge matching treatment so the split stays balanced, then re-measuring both
metrics together. If that holds, the 4-7x symmetry improvement is available WITH drainage. It
plausibly connects to the +-1.0 clamp lever, since both are about how much transport one tick is
allowed to do.

Perf was not measured. July saw ~8% on an older solver.
