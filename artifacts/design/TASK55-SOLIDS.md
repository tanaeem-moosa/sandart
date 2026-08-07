# Task #55 (SOLIDS) — fixing the granular lateral driving law

**Worktree:** `/home/deck/projects/sandart/.claude/worktrees/agent-ad1f5d94ec4ce85bf`
(branch `worktree-agent-ad1f5d94ec4ce85bf`). All changes are in
`sandart-sim/src/physics.rs`, uncommitted (not pushed, not committed to main, per instructions).

> **UPDATE (Task #59, read this first):** the discharge-rate finding below ("14x rate for 6x
> mass") does not hold up. It was a measurement artifact — see **"Task #59 update"** near the
> bottom of this file for the corrected measurement, the investigation the coordinator asked for
> before any fix, and the demonstration rig for the lateral fix under real overburden. No new
> production code was written this round; everything below the update marker is new diagnostics
> and corrected numbers only. Read the original Task #55 material below first for context, then
> the update.

## Important scope note, read first

The task brief's "evidence" section describes `multiplicative_lateral_gate`, a second gated
lateral-head form living in the **main checkout** (`/home/deck/projects/sandart/sandart-sim/src/physics.rs`),
which two other agents are live-editing. **That code does not exist in this worktree** — my
worktree branched before it was added, and it is uncommitted work in someone else's live session,
so I did not port or touch it (isolation is the point of the worktree split). Everything below is
diagnosed and fixed against **this worktree's own, already-shipped, default-on mechanism**: the
Stage C "combined liquid + granular share" lateral edge in `settle_tick`, which already has a
`tau` (yield stress) gate — `GRANULAR_TAU_SCALE * PROP_THRESHOLD * granular_share` — compared
against `head_a - head_b_full`. The conflation the task brief describes (comparing a
depth-independent threshold against a depth-*dependent* driving quantity) is structurally present
in this mechanism too, independent of whether the specific `multiplicative_lateral_gate` bug the
parent measured ever ships. I fixed it here, gated, default off.

## What the current math actually does (read before touching anything)

- **`column_depth` / `recompute_column_depth`** (`physics.rs` ~L770+): a top-down running sum,
  one row at a time, of "how much resting material sits above this cell" — literally a hydrostatic
  column integral. This is the correct construction for a **liquid**: real hydrostatic pressure
  grows linearly with depth, unboundedly. It is not, on its own, appropriate for granular material,
  which is exactly why `janssen_effective_depth` exists.

- **`janssen_effective_depth()` / `JANSSEN_DEPTH_SCALE`**: transforms raw `column_depth` through
  `JANSSEN_DEPTH_SCALE * (1 - exp(-depth / JANSSEN_DEPTH_SCALE))`, blended by liquidity so a
  liquid cell gets the identity (unbounded) transform and a granular cell gets the saturating one.
  This is a real, if `NOT MEASURED` (per its own doc comment), attempt at Janssen: it prevents the
  *vertical stress* term from growing without bound for granular material.

- **`LATERAL_PRESSURE_SCALE` / `k_of_liquidity` / `LATERAL_EARTH_PRESSURE_K`**: `k_of_liquidity`
  blends between `K = 1` (liquid, isotropic, full lateral transmission) and `K = 0.45` (granular,
  Jaky's `K0 = 1 - sin(phi)` estimate) — a legitimate earth-pressure-at-rest coefficient. Applied
  at the lateral edge as `head_a += k_a * LATERAL_PRESSURE_SCALE * janssen_effective_depth(...)`.

- **What currently produces the angle of repose** (found before changing anything, per the task's
  own instruction): the Stage C "Combined liquid + granular share" lateral edge in `settle_tick`
  (search `GRANULAR_TAU_SCALE`), NOT the old CA's `0.20` avalanche valve (that valve is now
  unreachable for any non-Oobleck material under gravity — see the "Stage C bail-out" comment right
  after this edge). The mechanism: `tau = GRANULAR_TAU_SCALE * PROP_THRESHOLD * granular_share`
  (`GRANULAR_TAU_SCALE = 1.0`, `PROP_THRESHOLD = 0.08` for DrySand) is compared against
  `head_a - head_b_full` inside `edge_sleeps` (hard gate: below `tau`, the edge sleeps, zero flux)
  and again inside `flux_edge_candidate` (`yielded = driving - tau`, Bingham/Coulomb-plastic
  post-yield damping). **This is where the conflation lives**: `head_a`/`head_b_full` already
  include the `k_of_liquidity * LATERAL_PRESSURE_SCALE * janssen_effective_depth(column_depth)`
  overburden term — a STRESS-shaped quantity that legitimately grows with how much is stacked on a
  column — compared directly against `tau`, a threshold calibrated purely as a bare GEOMETRIC
  SLOPE (`PROP_THRESHOLD` is literally the old CA's `geom_slope` cutoff — see `GRANULAR_TAU_SCALE`'s
  own doc comment). Comparing a depth-growing quantity against a depth-independent threshold is a
  liquid-appropriate (hydrostatic) yield test wearing a granular threshold's clothes.

- **Janssen vs. repose — are they conflated?** Yes, confirmed by reading the code, exactly as the
  brief asked me to check. Janssen saturation (`janssen_effective_depth`) legitimately answers "how
  much of THIS column's own weight is carried by wall/grain friction instead of straight down" — a
  confined-silo question about stress *magnitude*. It has nothing to say about whether a free
  surface is steeper than its angle of repose, which is a question about slope *ratio*
  (Mohr-Coulomb: yields when shear/normal exceeds `tan(phi)`, a depth-independent ratio). The
  shipped code feeds Janssen's answer into the very same value the repose threshold gates on, so
  the two get conflated in the yield decision. My fix (below) is exactly the separation the task
  brief asks for.

## What I changed

Added `granular_yield_gate` (`#[cfg(test)]` thread-local + `#[cfg(not(test))]` hardcoded-`false`
twin, same pattern as `fresh_overburden_gate`/`upstream_wake_gate`/`pressure_gate`), default OFF.
`physics.rs` ~L1794 for the gate module; the call site is in `settle_tick`'s Stage C lateral edge,
~L4339-4380.

Gate OFF (default): `sleep_driving = head_a - head_b_full` — bit-identical to the pre-existing
tree in every respect (confirmed: full test suite passes exactly as before, all new diagnostics
below reproduce the shipped numbers under this branch).

Gate ON: `sleep_driving` becomes a bare geometric slope instead —

```rust
let eta_a = h_a_frozen + gravity_dir.x * GRAVITY_HEAD_SCALE + column_depth[center_idx]; // RAW, un-weighted
let eta_b = h_b_frozen + column_depth[nb_idx];
sleep_driving = eta_a - eta_b + dispersion;
```

`column_depth` here is used **raw** — no `k_of_liquidity`, no `janssen_effective_depth` — because
`eta` is meant to answer a purely geometric question ("how tall is this column, physically"), and
`column_depth` is already stored in genuine height units (a running sum of resting cell heights),
so no unit conversion is needed. `tau` (the SAME existing threshold, unmodified — no new constant,
per the task's constraint) gates this bare slope instead of the weighted head.

Critically, `flux_tau` (what `flux_edge_candidate` actually subtracts once an edge is awake) is
**left at `tau`, unchanged, in both branches** — see the long comment at the call site and the
gate's own doc comment for why. My first draft zeroed it on the theory that "the yield decision was
already made in slope-space, so subtracting `tau` a second time double-gates." That was wrong: it
measurably **over-drove** every awake edge (Bingham/Coulomb-plastic post-yield flow is supposed to
be proportional to the *excess* over yield stress, not the bare driving stress). On the shallow
repose rig, where the geometric and weighted criteria agree almost exactly (`column_depth ~= 0`
there — see below), that alone dropped the measured repose angle from ~5.07° to ~2.34°, a pure
confound with nothing to do with depth-independence. Fixed by leaving `flux_tau = tau` always: the
gate now changes **whether** an edge gets a chance to move (depth-independent, fixed), never **how
much** it moves once it does (unchanged, still the existing tuned rate law). This is documented in
detail in the gate's own doc comment (`physics.rs` ~L1794-1857) so a future reader doesn't
re-discover the same mistake.

No new physical constants were introduced. Everything reused: `tau`, `column_depth`,
`GRAVITY_HEAD_SCALE`, `dispersion` (so the "ragged, grainy heap surface" texture is preserved,
since dispersion still perturbs the yield test, just at the geometric level now).

## Measurements

### `test_dry_sand_has_angle_of_repose` — gate OFF vs ON

Reproduced via a new diagnostic, `diag_task55_granular_yield_gate_depth_independence`
(`physics.rs` ~L8970), since the real test doesn't take a gate parameter. Runs CASE 1 (steep
collapse) and the NON-VACUITY ANCHOR (DrySand vs. Water) at the test's own shallow pile, both gate
states:

```
gate_on=false shallow: initial=0.3500 (19.29 deg) final=0.0886 (5.07 deg) flow=412.70
gate_on=true  shallow: initial=0.3500 (19.29 deg) final=0.0886 (5.07 deg) flow=412.70   <- identical
ANCHOR gate_on=false shallow: DrySand=0.0652 (3.73 deg) Water=0.0000 (0.00 deg)
ANCHOR gate_on=true  shallow: DrySand=0.0652 (3.73 deg) Water=0.0000 (0.00 deg)          <- identical
```

The real `test_dry_sand_has_angle_of_repose` itself (gate off, unmodified) still passes: sand
settles to ~5.07°, holds a built-shallower pile, re-establishes its angle after a peak deposit, and
the non-vacuity anchor shows DrySand (3.73°) clearly above Water (0.00°). No regression from adding
the gate; the gate is a genuine no-op at its default.

I also tried to build a deliberately **deep** pile (20x the area, same slope) on the same rig to
show the fix mattering more strongly at depth, expecting the shipped mechanism's depth-dependence
to show up there. It did not, gate on/off were again nearly identical
(`final=0.2443 (13.73 deg)` both branches) — **and this is a REASONED, not a clean MEASURED,
finding of "no bug here"**: `ReposeRig::build` writes the whole pile silhouette into a *single*
grid row (`sim.hm.data[ramp_row * w + x] = hgt`, with `hgt` allowed to exceed capacity), so
`column_depth` at the measured row is always ~0 regardless of how much total mass the pile holds —
there is never any material genuinely *stacked above* the measured row in this construction. The
rig cannot exercise the conflation at all, in either direction; the deep-pile run I added mostly
demonstrates that (worth keeping as a diagnostic, but it is not evidence the conflation is harmless
at real depth, only that this specific rig can't see it). A back-of-envelope check for a genuinely
self-similar straight-sided wedge (adjacent columns at constant local slope `s`, both filled to
capacity below the surface) suggests the weighted term's *Janssen-saturated* difference actually
**shrinks** relative to the raw slope difference as depth grows, converging toward (not diverging
from) the geometric criterion — i.e. for a perfectly straight-sided pile the practical severity of
the conflation may be small by construction, and the mechanism is more clearly wrong on
architectural grounds (a repose criterion that is not provably depth-independent) than on any
dynamic scenario I was able to build and measure in the time available. I did not find a scenario
in this simulator where the shipped additive path visibly mis-flattens a deep pile the way the
task's `multiplicative_lateral_gate` evidence showed for that other (unavailable-to-me) mechanism.
**State plainly: this specific severity claim is reasoned, not measured.**

### `test_liquid_flowing_liquid_does_not_stand_in_walls` — gate OFF vs ON

New diagnostic `diag_task55_granular_yield_gate_liquid_walls_regression` (`physics.rs` ~L7060),
reproducing the real test's construction (pure Water, Hourglass, 400 ticks) under both gate states
in one process:

```
gate_on=false voids@120=60 voids@160=6 total=11049 mass 883.000 -> 883.000
gate_on=true  voids@120=60 voids@160=6 total=11049 mass 883.000 -> 883.000
```

**Bit-identical**, not just close. This is expected and *provable in advance*, not a coincidence:
`tau == 0.0` for pure liquid (`granular_share == 0.0`) in both branches of the gate, so
`sleep_tau == 0.0` either way and `edge_sleeps`' `driving.abs() <= tau` degenerates to
`driving.abs() <= 0.0` regardless of which driving expression is used — any nonzero drive wakes the
edge under both formulas. Confirmed here against the real solver rather than left asserted only in
a doc comment. The real (unmodified) `test_liquid_flowing_liquid_does_not_stand_in_walls` passes,
gate off, as shipped.

### Hourglass discharge rate vs. fill height (Beverloo/Janssen signature)

No existing test measured this, so I wrote one: `diag_task55_hourglass_discharge_rate_vs_fill_height`
(`physics.rs` ~L6266). Fills the top chamber of an Hourglass mask to two very different depths
(a thin skim vs. ~90% of the chamber, ~6x the mass), lets a transient settle for 150 ticks, then
measures the drain rate (mass arriving in the bottom chamber per tick) over a 200-tick window:

```
gate_on=false shallow: fill_mass=131.0 drain_rate=0.00041/tick
gate_on=false deep:    fill_mass=795.0 drain_rate=0.00572/tick  (mass 6.07x, rate 14.00x)
gate_on=true  shallow: fill_mass=131.0 drain_rate=0.00041/tick
gate_on=true  deep:    fill_mass=795.0 drain_rate=0.00520/tick  (mass 6.07x, rate 12.74x)
```

**This is a genuine, measured, remaining problem, and my gate does NOT fix it.** A real granular
hourglass's drain rate is set by neck geometry alone (Beverloo), independent of fill height,
because Janssen saturation shields the neck from the growing weight above. Here, 6x the mass drains
~13-14x faster in EITHER gate state — closer to Torricelli's law (hydrostatic head driving faster
efflux, i.e. liquid behaviour) than to Beverloo. My gate barely moves it (14.00x -> 12.74x) because
**this measurement is dominated by the VERTICAL (gravity-aligned) edge**, not the lateral edge my
gate touches: the vertical overburden bonus is `VERTICAL_PRESSURE_SCALE * janssen_effective_depth(...)
* k_of_liquidity(...)`, capped by `VERTICAL_PRESSURE_CAP_MULT`, applied unconditionally (not gated
at all, always on in production) in phase 0. It has no yield-stress/`tau` concept whatsoever —
gravity is assumed to always want to pull material down, so there is no "does this edge move at
all" question there the way there is laterally, only "how fast." If that bonus is what is making
deep material accelerate through the neck, it is the same class of liquid-appropriate mistake the
task describes, just on the other edge. **I did not fix this** — it is outside the specific
lateral mechanism the task's own evidence pointed at, touching it would mean modifying an
always-on, ungated mechanism (`VERTICAL_PRESSURE_SCALE`) that Task #54 already tuned and shipped,
and I ran out of budget to design, gate, and validate a second fix safely in this pass. Flagging it
explicitly as unresolved, with a real measurement, rather than silently leaving it out.

### `cargo run --release --example bench_sandfall -- --ticks 600 --materials water,drysand`

The gate is `#[cfg(test)]`-only, per the task's own design constraint (production/`--release`
examples must pay nothing for it) — `granular_yield_gate` **does not exist** in the `bench_sandfall`
binary at all, so there is no "gate on" run to produce for this command; only "gate off" (the only
reachable state) exists outside `cargo test`. Since `is_enabled()` is a hardcoded-`false` function
outside `#[cfg(test)]`, the compiler should fold the whole gate branch away — the generated code
for the non-test `settle_tick` should be byte-for-byte the same as before this change (a REASONED
claim about codegen, not an independently verified disassembly diff). Ran it to confirm no
build/behaviour/perf regression from the code changes, 512x512 (production resolution), 600 ticks
+ 20 warmup:

```
material      budget     ms/tick     ticks/s  mass_rel_err     must budgeted    stale
------------------------------------------------------------------------------------
Water           1024     14.4517        69.2       2.12e-7    129.5     86.8     26.9
Water            256     16.6084        60.2       2.20e-7    129.9     62.5     26.9
Water             32     14.9323        67.0       2.05e-7    122.2      0.9     30.0
DrySand         1024     13.3526        74.9      8.14e-10    104.4     78.4     27.8
DrySand          256     13.2993        75.2       1.06e-9    104.4     75.7     27.8
DrySand           32     12.2466        81.7      2.27e-10    102.8      0.0     30.6
```

Runs clean, mass conservation error negligible (`~1e-9` for DrySand, `~1e-7` for Water), ms/tick in
the same range as expected for this scene. Nothing here indicates a regression, consistent with the
gate being fully compiled out in this build.

## Test suite results (this worktree)

- `cargo build -p sandart-sim --release` — clean (2 pre-existing dead-code warnings, unrelated).
- `cargo test -p sandart-sim --release --lib` — 96 passed, 1 failed (**expected**:
  `test_water_blob_stays_left_right_symmetric_under_gravity`, the documented pre-existing marker
  bug — untouched, not weakened, scan order unchanged), 23 ignored. This is GREEN per the task's
  own definition.
- `cargo test -p sandart-sim --release --test perfect_simulation_determinism` — 2/2 passed.
- `cargo test -p sandart-sim --release --test fresh_pressure_field_toggle` — 2/2 passed.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release` — clean.

## What remains unresolved

1. **Vertical/discharge-rate liquid-appropriateness** (measured above): drain rate through a fixed
   neck scales ~14x for a 6x mass increase, in both gate states. This is very likely the same class
   of bug on the *vertical* edge (`VERTICAL_PRESSURE_SCALE`/`janssen_effective_depth` in phase 0,
   Task #54), which has no yield-stress concept at all today. Not fixed here; needs its own gated
   design and validation pass.
2. **Deep-pile lateral severity is reasoned, not measured** (see above): I could not build a dynamic
   scenario in this suite's own test rigs that shows the lateral conflation producing a large,
   visible repose-angle error at depth. The fix is still correct on architectural grounds (a repose
   criterion should be provably depth-independent, not "small enough in practice because Janssen
   happens to saturate"), but I want to be explicit that the severity claim in the task brief's own
   evidence (attached to `multiplicative_lateral_gate`, a mechanism not present in this worktree)
   is not something I independently reproduced against the additive default path.
3. I did not attempt to port or reconcile with `multiplicative_lateral_gate`/`mult_lateral_conveyance`
   from the main checkout at all (out of scope for an isolated worktree, and it was still being
   actively edited by other agents while I worked). If that mechanism is merged later, the same
   slope-vs-weighted-magnitude decoupling principle in `granular_yield_gate` should generalise to
   it directly (its own `eta`/`conveyance` split already separates the two questions in spirit; the
   remaining unit-scale bug I noticed in passing — `local_fill` rescaled by `depth_scale` before
   being raised to `MULT_LATERAL_CONVEYANCE_EXPONENT`, inflating `conveyance` by `depth_scale^1.5`
   at any non-production test resolution — looked like the more likely proximate cause of the
   reported catastrophic (19.29° -> 0.41°) failure than the Janssen/repose conflation per se, but I
   did not verify this since I never had that code in front of me to instrument).

## Files touched

- `sandart-sim/src/physics.rs`:
  - New `granular_yield_gate` module (~L1794-1857, adjacent to the other `#[cfg(test)]` gates).
  - Modified Stage C lateral edge in `settle_tick` (~L4339-4392) to compute `sleep_driving`
    through the gate, leaving `flux_tau`/the rest of the edge untouched.
  - Three new diagnostics: `diag_task55_hourglass_discharge_rate_vs_fill_height` (~L6266),
    `diag_task55_granular_yield_gate_liquid_walls_regression` (~L7060),
    `diag_task55_granular_yield_gate_depth_independence` (~L8970).

---

## Task #59 update — discharge rate investigation, and demonstrating the lateral fix under real overburden

Three jobs, in the priority order given: (1) investigate and, if warranted, fix the discharge-rate
defect; (2) keep any fix gated; (3) build a rig that actually exercises `granular_yield_gate`.
**No production code changed this round.** Job 1's own measurement, done properly, did not
support a fix — see below, reported plainly rather than quietly patched around. Job 3 produced two
new rigs and a genuine, if modest and non-dynamically-significant, divergence.

### Job 1: the discharge-rate defect does not hold up under correct measurement

**Instrumentation first, as asked.** `diag_task59_vertical_bonus_saturation_curve`
(`physics.rs` ~L9150) calls `janssen_effective_depth`, `k_of_liquidity`, and the
`VERTICAL_PRESSURE_CAP_MULT` clamp directly — the exact functions phase 0's vertical edge calls —
across a sweep of raw `column_depth` from 0 to 300. The answer to "where does the curve flatten,
and why":

```
liquidity=0.00 (granular, k=0.4500):
  raw_column_depth=0.00  ->  total_head = 1.00x base
  raw_column_depth=0.20  ->  total_head = 1.45x base
  raw_column_depth=0.45  ->  total_head = 2.00x base  (CAP-LIMITED from here on)
  raw_column_depth=300.0 ->  total_head = 2.00x base  (identical to depth=0.45)

liquidity=1.00 (liquid, k=1.0000):
  raw_column_depth=0.00  ->  total_head = 1.00x base
  raw_column_depth=0.20  ->  total_head = 2.00x base  (CAP-LIMITED from here on)
  raw_column_depth=300.0 ->  total_head = 2.00x base  (identical to depth=0.20)
```

**It flattens almost immediately (raw depth ≈ 0.2–0.45, well under one cell) — but NOT because of
Janssen's own saturating shape.** It flattens because `VERTICAL_PRESSURE_CAP_MULT`'s hard
CFL-style clamp (`raw_bonus.min(base_head.abs() * VERTICAL_PRESSURE_CAP_MULT)`, `CAP_MULT = 1.0`)
engages first, at a depth where Janssen's own curve (which for granular doesn't even reach half its
`JANSSEN_DEPTH_SCALE = 24` plateau until raw depth ≈ 24) is still deep in its unsaturated, nearly
linear regime. Janssen's shape is essentially irrelevant to what actually caps this term in the
shipped code — the blunt 2x cap does all the flattening, for **both** materials, at almost the same
trivial depth. Directly answers the coordinator's candidates: not "`JANSSEN_DEPTH_SCALE` too
large," not "saturation applied to the wrong quantity" — it's that `VERTICAL_PRESSURE_CAP_MULT`
pre-empts Janssen entirely before Janssen's shape ever gets a chance to matter.

**Consequence, verified against the real solver.** If the vertical driving head is already flat
past a trivial depth for both materials, the discharge rate through a neck (which depends on that
head) should already be close to depth-independent for both — which directly contradicts my
original Task #55 finding. So I went back to that finding and instrumented the actual run:

`diag_task59_hourglass_discharge_trajectory` (`physics.rs` ~L6257) traces `bottom_sum` every 25
ticks for the same "shallow" and "deep" fills the original diagnostic used. **The shallow fill
(131 mass) had already drained to `top_remaining=0.53` by tick 150** — the ORIGINAL diagnostic's
own `SETTLE_TICKS` value, i.e. its 200-tick "measurement window" (ticks 150–350) started AFTER the
shallow reservoir was already empty. The deep fill (795 mass) was also nearly fully drained
(`top_remaining=2.13`) by tick 150. **The original "14x rate for 6x mass" figure compared two
near-empty reservoirs' residual settling noise, not real discharge.** That is a methodology bug in
the diagnostic I wrote for Task #55, not a property of the solver.

**Corrected measurement.** `diag_task59_hourglass_discharge_rate_during_active_flow`
(`physics.rs` ~L6353) measures rate during the genuinely active phase (a fixed early window, with
the remaining reservoir mass reported alongside so a fill that's ALSO running dry within the window
is visible rather than silently averaged over), across four fill fractions and both materials:

```
material=DrySand frac=0.15 fill_mass= 75.0 rate=0.7953/tick  remaining_at_tick15=  7.2  <- already draining dry, discard
material=DrySand frac=0.30 fill_mass=191.0 rate=8.1323/tick  remaining_at_tick15= 36.8  <- partly contaminated, discard
material=DrySand frac=0.60 fill_mass=469.0 rate=11.7437/tick remaining_at_tick15=278.0  <- healthy reservoir
material=DrySand frac=0.90 fill_mass=795.0 rate=11.8564/tick remaining_at_tick15=602.0  <- healthy reservoir
                                                              ratio: 11.86 / 11.74 = 1.01x for 1.7x mass

material=Water   frac=0.60 fill_mass=469.0 rate=7.5245/tick  remaining_at_tick15=349.2
material=Water   frac=0.90 fill_mass=795.0 rate=7.5245/tick  remaining_at_tick15=675.2
                                                              ratio: EXACTLY 1.0000x for 1.7x mass
```

`peak_1tick_rate` is identical across every fill fraction within a material (19.0 for DrySand,
9.592 for Water) — a hard per-tick transfer ceiling (donor/acceptor mass-conservation clamp at the
neck), independent of depth. Tick-by-tick traces (printed by the same diagnostic) confirm the
0.60/0.90 windows are genuinely in a repeating, saturated oscillation band (a parity/checkerboard
artifact in the per-tick values, consistent with this file's own documented order-dependence notes
elsewhere — see `test_water_blob_stays_left_right_symmetric_under_gravity`'s doc comment — but the
*band average* is stable): DrySand's tick 14-26 average is ~11.67 at frac=0.60 vs ~11.72 at
frac=0.90.

**Answer to Job 1: once measured correctly (an actively-fed reservoir, not one running dry mid-window),
DrySand's discharge rate is already within ~1% of depth-independent across a 1.7x mass range, and
Water's is bit-for-bit identical.** The headline multiplier the coordinator asked for: **~1.01x for
granular (target: close to 1x — already met)**. I did not implement a fix, because there is no
defect here to fix; implementing one anyway (a liquidity split, or a `JANSSEN_DEPTH_SCALE` retune)
against a measurement this flat would be tuning a constant to a target that's already satisfied,
which the task's own instructions rule out. **This is my measurement pointing somewhere other than
the coordinator's premise, stated explicitly rather than quietly worked around.**

**An unresolved, genuinely surprising side-finding, explicitly flagged rather than acted on.** The
coordinator's framing assumed "deep material falls faster" is a currently-working, deliberately
shipped LIQUID behaviour that a granular-only fix must not break. My measurement says it is
**not currently happening for either material** beyond the same trivial ~0.2–0.45 raw-depth
threshold — Water's rate is exactly flat too (7.5245 at both frac=0.60 and frac=0.90). If Task #54
intended unbounded (or at least much-further-growing) depth-dependence for liquid, that intent is
being defeated today by `VERTICAL_PRESSURE_CAP_MULT`'s low, material-agnostic cap — which would
mean the "tension" the coordinator described (granular wants saturation, liquid wants continued
growth) isn't actually live in the current build for either side to begin with. I have **not**
touched `VERTICAL_PRESSURE_CAP_MULT` or anything on the vertical edge — this is Task #54's
mechanism, the coordinator explicitly asked not to have this tension resolved unilaterally, and my
own priority-1 finding is "no fix needed for granular," which makes touching a shared constant even
less appropriate. Flagging for whoever owns Task #54's liquid-depth behaviour next: if "deep water
falls faster" is still wanted, the fix is raising or removing `VERTICAL_PRESSURE_CAP_MULT` for the
liquid share specifically (again via a `k_of_liquidity`-style split, not a flat retune) — a
different, separate piece of work from anything in Task #55/#59.

### Job 3: demonstrating `granular_yield_gate` under real overburden

Two new rigs, in the order I actually built them (including the one that didn't work, since the
coordinator asked for the honest result either way).

**Attempt 1 — buried step (did not isolate the conflation).**
`diag_task59_buried_step_overburden_conflation` (`physics.rs` ~L9284) builds a TALL block of
packed DrySand directly beside a SHORT block, both resting on the same floor, and reads the
lateral edge at a row buried below both blocks' own surfaces (both sides locally at capacity,
`h_a - h_b = 0` exactly). Single-tick numeric read:

```
h_a=1.5000 h_b=1.5000 (local slope=0.0000)  raw_column_depth: LEFT=558.00 RIGHT=48.00
tau=0.0800 | SHIPPED weighted_diff=7.3081 (EXCEEDS tau) | GATED geometric_diff=510.0000 (EXCEEDS tau)
```

Both readings say "yields," just by very different margins. This construction turned out to be a
poor test: because LEFT and RIGHT are IMMEDIATE lateral neighbours (`x` and `x+1`) with surfaces
~85 rows apart, comparing them at depth is comparing a genuine, one-cell-wide buried CLIFF, not a
gentle slope with real overburden — both mechanisms correctly call a buried cliff unstable, so
nothing about the conflation specifically was shown. Kept in the suite as a diagnostic and reported
here as a dead end, per the instruction to say plainly when a rig doesn't work.

**Attempt 2 — smooth wedge (the correct test, and it worked).**
`diag_task59_smooth_wedge_conflation_vs_depth` (`physics.rs` ~L9432) builds a genuine self-similar
wedge — constant local slope `s = 0.9 * tau` (10% below DrySand's own repose threshold, so by the
repose criterion this pile must be stable EVERYWHERE, not just at its visible surface) — filled
across 15 real rows (not `ReposeRig`'s single-row silhouette hack), wide enough (`w=428`) that a
lateral edge deep inside the wedge body, away from the outer flank, can be read at several depths
below the peak. Single-tick numeric read at 5 depths:

```
layer= 2 (near surface): local=0.0000  raw_depth diff=-1.7944 | SHIPPED=-3.8901 | GATED=-1.7944  (ratio 2.17x)
layer= 5:                local=0.0000  raw_depth diff=+0.0000 | SHIPPED=+0.0000 | GATED=+0.0000  (flat tread, both agree)
layer= 8:                local=0.0000  raw_depth diff=+0.0000 | SHIPPED=+0.0000 | GATED=+0.0000  (flat tread, both agree)
layer=11:                local=0.0000  raw_depth diff=+0.0000 | SHIPPED=+0.0000 | GATED=+0.0000  (flat tread, both agree)
layer=14 (near base):    local=0.0000  raw_depth diff=-1.7944 | SHIPPED=-2.4839 | GATED=-1.7944  (ratio 1.38x)
```

Two findings, both real:

1. **On the "flat treads" (most of the wedge body), the shipped and gated criteria are IDENTICAL,
   both exactly zero, both correctly stable.** A slope this shallow (`s ≈ 0.072`) needs ~14 cells
   of horizontal run to accumulate one row of height, so `column_depth` is a step function of `x`
   with long flat runs — and on those flat runs, `column_depth` is literally equal for adjacent
   columns, so there is nothing for the weighted term to amplify. The conflation has no purchase
   here.
2. **At the "risers" (where the staircase discretization of a shallow slope forces a full one-row
   jump — an inherent grid-quantization artifact of representing `s ≈ 0.072` on integer rows, not a
   defect in either mechanism), BOTH criteria agree the edge should yield (a genuine local step of
   1.79 units is far above any plausible `tau`), but the SHIPPED magnitude overshoots the GATED one
   — and by MORE near the surface (2.17x at layer 2) than at depth (1.38x at layer 14).** This is
   the opposite depth trend from my original Task #55 guess ("deep piles over-yield"). It matches
   the analytic prediction once worked through properly: the weighted term's depth contribution is
   `~ K * SCALE * s * janssen'(depth)`, proportional to Janssen's own DERIVATIVE, which is largest
   near the surface (`janssen'(0) = 1`) and decays with depth (`janssen'(depth) = exp(-depth/24)`)
   — so the shipped mechanism's error is worst just below the surface and self-corrects with depth,
   not the reverse.

**Dynamic follow-up, and the honest limit of this finding.** Running the identical wedge for 40
real ticks, gate off vs on:

```
gate_on=false: total_flow over 40 ticks=276529.216 mass_delta=+0.0000
gate_on=true:  total_flow over 40 ticks=276526.586 mass_delta=-0.0020
```

**Statistically identical** (0.001% apart). The static, per-edge magnitude divergence measured
above (2.17x / 1.38x overshoot at the two riser rows) is real and directly attributable to the
conflation, but it did not show up as a detectable difference in this AGGREGATE dynamic measure —
the wedge's total flow is dominated by the many other risers and general settling across a
428-wide grid, which swamps the signal from the handful of edges where the two mechanisms actually
disagree in magnitude. **Stated plainly, as asked: under real overburden, the corrected comparison
does produce a measurable, real, instantaneous divergence from the shipped one (confirmed by direct
numeric read against the live solver, not just the formula in isolation) — but I could not show
that divergence changing the AGGREGATE settling outcome of a realistic pile at the depths and tick
counts tested here.** I do not read this as evidence the fix is unnecessary — the per-edge
divergence is real, it is architecturally exactly the bug the task describes (a depth-independent
threshold compared against a depth-shaped quantity), and the shipped mechanism's error is
concentrated exactly where surface texture/dispersion already lives (just below the surface, where
`GRAVITY_LOCK_CHANCE`/`DISPERSION_TAU_FRAC` also operate) — but I want to be honest that I did not
find a scenario where it changes a whole-pile outcome by an amount larger than this solver's own
background noise.

### Required measurements, gate off vs on (unchanged production code this round)

**`test_dry_sand_has_angle_of_repose`** (real test, gate off — the only state it runs at):

```
CASE 1 (steep): initial=0.3500 (19.29 deg) final=0.0886 (5.07 deg) total_flow=412.70
NON-VACUITY ANCHOR @450 ticks: DrySand=0.0652 (3.73 deg) Water=0.0000 (0.00 deg)
CASE 3 (at angle): initial=0.0886 final=0.0760 (4.34 deg)
CASE 2 (shallow): initial=0.0532 final=0.0534 (3.06 deg) s_measured=0.0886 (5.07 deg)
CASE 4 (deposit on peak): peak_after_deposit=2.9241 peak_after_resettle=1.2334 flank_slope=0.0972 (5.55 deg)
```

Passes. Gate-on numbers (via `diag_task55_granular_yield_gate_depth_independence`, unchanged from
the original Task #55 report since nothing about the gate itself changed this round): bit-identical
to gate-off on this shallow rig (5.07° both; anchor 3.73° vs 0.00° both) — expected, and reconfirmed
this round rather than assumed.

**`test_liquid_flowing_liquid_does_not_stand_in_walls`**: passes, gate off (unmodified). Gate on/off
regression check (`diag_task55_granular_yield_gate_liquid_walls_regression`): still bit-identical
(`voids@120=60 voids@160=6 total=11049`, both gate states) — reconfirmed this round.

**Hourglass discharge multiplier (headline number)**: **~1.01x for DrySand, 1.00x (exact) for
Water**, at a 1.7x mass ratio, once measured during genuinely active flow — see Job 1 above for the
full derivation and why the original "14x" number is retracted.

**`bench_sandfall --ticks 600 --materials water,drysand`, 512x512**: rerun this round for
completeness (no production code changed, so no behavioural difference expected):

```
material      budget     ms/tick     ticks/s  mass_rel_err     must budgeted    stale
------------------------------------------------------------------------------------
Water           1024     17.3113        57.8       2.12e-7    129.5     86.8     26.9
Water            256     17.3485        57.6       2.20e-7    129.9     62.5     26.9
Water             32     16.4176        60.9       2.05e-7    122.2      0.9     30.0
DrySand         1024     13.9280        71.8      8.14e-10    104.4     78.4     27.8
DrySand          256     13.6108        73.5       1.06e-9    104.4     75.7     27.8
DrySand           32     12.5030        80.0      2.27e-10    102.8      0.0     30.6
```

`mass_rel_err` is bit-identical to the Task #55 report's own run (deterministic, as expected since
this build did not change); `ms/tick` differs by machine noise only, confirming no regression.

### What remains unresolved (Task #59)

1. **Liquid's "deep falls faster" is not currently happening either** (see the side-finding above)
   — a real, measured gap between Task #54's stated intent and current behaviour, not something I
   fixed (out of scope for a granular-focused task, and touching the shared cap without the
   coordinator's sign-off is exactly the unilateral move I was told not to make).
2. **The lateral conflation's dynamic significance is still an open question.** The static
   divergence is real and measured; I could not push it into a measurable whole-pile dynamic effect
   within this session's time budget. A cleaner future test might isolate a SINGLE riser edge's own
   local flux (not the whole wedge's aggregate flow) to see the effect without 428 columns of
   background noise diluting it.
3. **`granular_yield_gate` remains default OFF, unmerged into production behaviour.** Given Job 1
   found no discharge defect and Job 3 found a real-but-small, aggregate-undetected divergence, I
   have not made a case strong enough on this round's evidence to recommend flipping the gate on by
   default; that remains a judgement call for whoever reviews this next, with the numbers above to
   decide from.

### Files touched (Task #59, in addition to Task #55's own list above)

- `sandart-sim/src/physics.rs`, all new `#[cfg(test)]` diagnostics, no production-path changes:
  - `diag_task59_hourglass_discharge_trajectory` (~L6257)
  - `diag_task59_hourglass_discharge_rate_during_active_flow` (~L6353)
  - `diag_task59_vertical_bonus_saturation_curve` (~L9150)
  - `diag_task59_buried_step_overburden_conflation` (~L9284)
  - `diag_task59_smooth_wedge_conflation_vs_depth` (~L9432)
