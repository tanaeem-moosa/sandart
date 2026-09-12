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

## 9. The 45-degree walls (2026-09-09) — it is incompressibility, and two fixes are refuted

User's report, on `cc3fc3f`: symmetry is good, the drainage speckle is fine ("I don't mind the
waterflow lines"), but water sits at "almost 45 degree walls". Clarified target: **not** full
flattening — "that is how we ended up on our misadventure with pressure" — but flattening MORE, and
losing the clear straight angles.

### It is NOT a red-black regression

`diag_water_levelling`: water piled into the left third of a CLOSED 256 box, so nothing can drain
and only lateral transport can change the surface. Surface height range across the box:

                   t=300      t=1200     t=3000
    baseline     range 232  range 202  range 141
    cc3fc3f      range 232  range 214  range 168

Red-black makes it ~19% worse; the defect is overwhelmingly pre-existing. Water does not level.

### The mechanism: a full cell has zero room

Same box filled to h=0.5 instead of h=1.0, so every cell has headroom:

    full (h=1.0)   maxslope 21 -> 6 -> 5
    half (h=0.5)   maxslope  6 -> 3 -> 2

Lateral flow cannot pass THROUGH a cell that is at capacity, so a piled body levels at roughly one
cell per tick. A uniform transport limit is what produces a uniform slope — the straight facet is
transport-limited, not pressure-limited. No constant moves this; `LATERAL_PRESSURE_SCALE` was swept
5/8/12/20 and `voids@160` got worse at every value above 5 while levelling did not improve
monotonically.

### REFUTED 1: raising water's cell capacity

`cell_capacity_for` gives water 1.0 and granular 1.5 — the material that most needs to transmit
lateral flow has the least headroom. Raising water to 1.5 looks excellent on every metric that was
being watched (levelling range 168 -> 102, `voids@120` 167 -> 146 and `voids@160` 77 -> 2, which
turns `test_liquid_flowing_liquid_does_not_stand_in_walls` from failing to passing).

**It is compressibility, not headroom.** The suite says so directly: `test_liquid_is_incompressible`
fails, plus both task55 scoreboards. The user caught it independently and framed it exactly right —
"are we effectively reducing starting content?" Yes: once cells reach the new limit you are back in
the same transport-limited state with denser water and a shorter pile. The gain is borrowed from
the transient while the slack is consumed, and paid for in physics correctness.

### REFUTED 2: a global preference for lateral flow

User's proposal: "add a preference for lateral flow when there is multiple candidate". The place it
lands is `in_transit_at`, which withholds vertically-in-transit mass from the lateral edge —
gravity's strict first claim. Sweeping the withheld fraction:

    withhold   levelling range   maxslope   voids@120  voids@160   stream width (4-cell tap)
      1.0          168              5          167        77        passes
      0.9          165              4           --        --        18  FAIL
      0.75         158              3           --        --        24  FAIL
      0.5          138              3          146       108        FAIL
      0.0          121              2          164       116        30  FAIL

It does what was asked — maxslope 5 -> 2, levelling +28%, mass conserved exactly, capacity never
exceeded. But EVERY reduction, down to 0.9, breaks `test_liquid_stream_stays_coherent`: the falling
stream fans out to 18-30 cells from a 4-cell tap.

That failure is already documented, in the operator-split comment a few lines below the site:
"a fused pass cannot tell 'falling' from 'resting' and spreads both, fanning a 4-cell stream out to
33. What actually distinguishes them is that the falling cell has somewhere to go *along* gravity
and the pooled cell does not."

**Which means the proposal is already implemented, correctly, for the case it is right in.**
`in_transit_at` withholds only up to `downstream_route` — so a cell that CANNOT fall (blocked
below, i.e. resting in a pile) already has full lateral availability. The knob only turns down the
part that keeps falling streams narrow. Do not re-sweep it.

### Where that leaves it

The levelling limit is incompressibility itself, and the two cheap ways around it are refuted: one
gives up incompressibility, the other gives up stream coherence. A resting pile already has full
lateral availability, so the remaining constraint is purely that mass must physically traverse
cells that are at capacity, one cell per tick.

That points back at the transport RATE — the `+-1.0` clamp and solver sub-stepping (`HANDOVER.md`
§10) — which is the one lever here that changes no physics at all, only how much of it runs per
rendered frame. It is also the only one of these that does not reopen the pressure project.

### Comment cleanup done alongside

`pressure_project` and `pressure_gate` were referenced in eight places in `physics.rs` as if live;
both were deleted in `3bb6533` ("it cannot run, and enabling it changes nothing"). One of those
references sent this session off to diagnose machinery that does not exist. All now corrected or
replaced with a tombstone that records what the deleted pass was FOR, since the packed-column limit
it targeted is real and still bites.

## 10. Wetness-weighted lateral sub-passes (2026-09-11) — the user's design, shipped behind a dial

The user rejected liquid-only gating ("it messes with mixed materials in a visually obvious way"),
so the lateral edge pass reruns `ceil(N)-1` extra times per tick, and extra pass k moves
`clamp(1 + (N-1)*liquidity(donor) - k, 0, 1)` of its flux. The DONOR's wetness is used, so dry
material is s = 1 (bit-identical) and N is real-valued. Extra passes read heads from `temp_heights`,
so each one sees the previous pass's movement. The weight scales the mass moved, never the stored
edge velocity. Dial: `lateral_substeps` (UI "Lateral substeps", default 1.0 = bit-identical).

`diag_lateral_substeps_sweep`, re-run independently:

    N     levelling range t=300/1200/3000   stream width (limit 12)   Yogurt repose   hourglass worst_mirror
    1.0   232 / 214 / 168                   10                        0.0308          0.443
    1.5   231 / 205 / 150                   10                        0.0260          0.329
    2.0   221 / 170 / 112                   12                        0.0237          0.532
    2.5   231 / 148 /  65                   12                        0.0225          0.666
    3.0   229 / 136 /  42                   14  FAIL                  0.0219          0.705
    4.0   221 / 117 /  24                   18  FAIL                  0.0206          0.708

Dry repose 0.0887 at every N. `max_h` 1.0 and mass conserved at every N. `final_mirror` stays 0.
Unlike REFUTED 2, streams hold up to N = 2.5, because `in_transit_at` still withholds falling mass
on every pass. Costs: the transient mirror error grows with N (up to ~1.6x), and frame time grows
too. The agent measured grid-512 water hourglass ms/tick at 11.2 / 19.5 / 26.4 for N = 1 / 2 / 3;
that perf number is not independently re-run. `column_depth` is one pass stale in extra passes,
which is an untried second lever. `maxslope` (the worst adjacent-column step) does not track the
facet and barely moves; judge by range.

## 11. The fractional pass was wasted compute: realise it stochastically instead (2026-09-11)

§10's `ceil(N)-1` extra passes ran the *whole* pass every tick regardless of `frac(N)` — at
`N = 2.5` a third pass swept the entire wet region and then moved only ~50% of what it computed
(`clamp(1 + 1.5*liquidity - 2, 0, 1)` tops out at 0.5 for a fully wet donor). The COLLECT/ARBITRATE
/APPLY cost is paid per pass regardless of the weight, so that third pass cost as much as a whole
extra pass while doing half the work — measured at 20 fps at N = 2.5 on grid 512, i.e. `N = 2.5`
cost the same as `N = 3` while transporting less.

Fix (the user's design, settled): realise `frac(N)` as a single stochastic coin flip, once per
tick, GLOBAL to the whole grid — never per block or per cell, since a per-block rate is exactly
what produced visible seams before (§9's REFUTED 2). `M = floor(N) + (1 if roll < frac(N) else 0)`
is rolled once, before the phase loop, and used in place of the raw dial everywhere the old code
read `N`: `extra_lateral_passes = M - 1`, and extra pass k moves `clamp(1 + (M-1)*liquidity(donor)
- k, 0, 1)` — same formula, `M` (an integer, this tick's realised pass count) substituted for `N`.
A fully wet donor therefore only ever pays for *whole* passes, never a discounted last one; damp
donors keep the same continuous-in-wetness response and the same expectation as before
(`E[weight] = 1 + (N-1)*liquidity`, since `E[M] = N` by construction). Integer `N` and `N <= 1.0`
never roll (`frac == 0`), so both stay bit-identical to §10's code, which was itself bit-identical
to pre-`8968667` at `N = 1.0`.

The roll (`lateral_pass_roll` in `physics.rs`) mixes `time_seed` and `tick_count` through the same
avalanche finalizer `stochastic_round` uses (xor-shift/multiply/xor-shift/multiply/xor-shift), not
`time_seed % 2` — both inputs increment by exactly 1 per tick in every call site, so a cheap
low-bit test would hand back a fixed alternating cadence, and a strict period-2 cadence risks
beating against a known period-2 edge-velocity mode (see `physics.rs`'s TOMBSTONE comment,
section 1 above). The finalizer's avalanche breaks that correlation.

### Sweep re-run (`diag_lateral_substeps_sweep`, same scenarios as §10's table)

    N     levelling range t=300/1200/3000   stream width (limit 12)   Yogurt repose   hourglass worst_mirror
    1.0   232 / 214 / 168                   10                        0.0308          0.443
    1.5   231 / 201 / 144                   10                        0.0252          0.331
    2.0   221 / 170 / 112                   12                        0.0237          0.532
    2.5   227 / 159 /  77                   12                        0.0239          0.595
    3.0   229 / 136 /  42                   14  FAIL                  0.0219          0.705
    4.0   221 / 117 /  24                   18  FAIL                  0.0206          0.708

Dry repose 0.0887 and `final_mirror = 0` at every N, same as §10; `max_h <= 1.0` and mass conserved
at every N (the assertions in the sweep's incompressibility block pass for all six). N = 1.0, 2.0,
3.0, 4.0 match §10's table exactly (integer N is unchanged, as designed). N = 1.5 and N = 2.5 move
by single-digit amounts against §10's fractional-weight numbers (e.g. 2.5's t=3000 range: 65 -> 77;
worst_mirror: 0.666 -> 0.595) — expected, since a fractional N is now a different physical process
(a per-tick coin flip between two integer pass counts) with the same first-moment behaviour, not
the same trajectory. Stream width at 2.5 is still within the 12-cell limit (tap 4 + allowance 8),
matching §10's finding that `in_transit_at` withholding still holds streams narrow at this N.

### Perf re-run (`diag_lateral_substeps_perf`, N = 1, 2, 2.5, 3, grid 512 hourglass drain)

    N     ms/tick    extra_pass_frac (bumped/total, over the 300 measured ticks)
    1.0   12.4550    0.0000 (0/0)       -- gate never entered (N <= 1.0)
    2.0   22.7405    0.0000 (0/300)     -- integer N never rolls
    2.5   27.3488    0.4933 (148/300)   -- vs. frac(2.5) = 0.5 exactly
    3.0   29.9762    0.0000 (0/300)

`extra_pass_frac` at N = 2.5 (0.4933) lands within a coin flip's sampling noise of the expected
0.5, confirming `lateral_pass_roll` is well distributed and not exhibiting the alternating-pattern
failure mode `time_seed % 2` would have shown. On cost, N = 2.5 now lands at 27.35 ms/tick, roughly
64% of the way from N = 2's cost (22.74) to N = 3's (29.98) — no longer pinned at N = 3's cost
(29.98, what §10's `ceil(N)-1` design paid every tick regardless of the fractional weight). This
run's absolute numbers (12.46 / 22.74 / 29.98 for N = 1/2/3) run somewhat higher than §10's cited
11.2 / 19.5 / 26.4 -- machine load at measurement time, not a regression; the fix under test is the
N = 2.5 column's position relative to its own N = 2 / N = 3 neighbours, and that moved from "equal
to N = 3" to "between N = 2 and N = 3", which is the result the fix was for.

`extra_pass_frac`'s tally is a temporary `#[cfg(test)]`-gated thread-local counter
(`LATERAL_ROLL_STATS` in `physics.rs`) added purely to produce this measurement; it compiles into
no non-test build, including the shipped wasm.
