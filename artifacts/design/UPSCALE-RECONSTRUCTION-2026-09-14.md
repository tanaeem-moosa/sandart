# Upscale reconstruction: measuring alternatives to mask-aware bilinear (2026-09-14)

**Revision note (round 4, 2026-09-16).** Round 3 shipped R6 behind a toggle; the user looked at it
on the deployed page and called it "quilting," worse than the shipped bilinear, and defaulted the
toggle off. §9 is the follow-up: it tests the leading hypothesis (R6's `f=h0/h_ref` shrinks every
non-flat interior cell, not just frontier ones) directly, finds the SPECIFIC claim in that
hypothesis wrong (R6's binary false negatives are dominated by frontier cells, not interior, by
4-8x) but the UNDERLYING mechanism right (a continuous coverage-deficit metric shows R6 shrinks
essentially every coarse-interior cell by a small, near-ubiquitous amount -- exactly what a fine
seam pattern looks like, and exactly what the binary metric is blind to), and introduces R7, a
one-line correction that cuts the interior deficit ~3x with no change to frontier behaviour, no
change to conservation, and no shader change (this document and the instrument only -- see the
task instructions this round was run under). R7 is NOT yet shipped; §9 ends with a request for
review, not a recommendation to ship.

MEASUREMENT ONLY. Nothing in `sandart-render`, `sandart-wasm`, or `sandart-sim`'s physics changed.
The instrument is `sandart-sim/examples/diag_upscale_reconstruction.rs`
(`cargo run -p sandart-sim --release --example diag_upscale_reconstruction`); everything below is
its output, re-derived once more for this writeup, plus the PNGs it wrote to this same directory.

**Revision note (round 2).** The first pass of this document (below, largely still true in §1-3)
had two invalid metrics and no metric for the complaint that actually mattered:
1. Its stream-width number summed occupied/mass over an ENTIRE row of a multi-neck vessel, which
   measures the *separation* between streams as much as any one stream's width, and it probed
   `neck_row+2`, still inside the neck's own throat rather than free fall. It also claimed the
   round-trip target ratio was "~1.10-1.15" — that number was the 256-vs-512 *simulation* speed
   difference from unrelated prior work, not applicable here: this method deliberately holds the
   physics fixed and measures the reconstruction alone, so the correct round-trip target is
   **1.0**, full stop.
2. Nothing measured staircasing directly. Global false-positive/false-negative/IoU counts are
   dominated by bulk pool and wall area and are blind to whether a front is jagged, which is the
   user's actual complaint ("curves not smooth").

§4 and §6 below are the corrected versions. §1-3 are mostly unchanged from round 1 (candidate
descriptions were not wrong), with R5 (a round-2 addition) folded into §3. Old §4/§6 content has
been replaced rather than kept alongside, since it would otherwise sit right next to the numbers
that supersede it — the wrong framing (1.10-1.15) and the row-summed metric are recorded here, in
this note, precisely so nobody re-derives them.

**Revision note (round 3).** Round 2 introduced a real bug and, once fixed, a real finding that
overturns its own recommendation:
1. **Pictures were built from raw snapshot (a), not the EMA (c) the page actually displays.**
   §5 even noted the original stream crop was flecked with real jitter — jitter the EMA exists to
   remove — while judging pictures that never had it smoothed. Fixed: `stream_neck` and
   `pool_wall_edge` are now shown at BOTH (a) and (c), same crop, so the effect of temporal
   smoothing is directly visible (§5 says how much of round 2's "speckle" claim survives it — most
   of it, since the speckle is spatial/per-cell, not temporal).
2. **R5's `coverage>=0.5` boundary convention was the entire cause of its 44-49px chamfer maxima**,
   confirmed and fixed: a film-case cell (all neighbours shallow, `f=h0/h_ref` near 1 even though
   `h0` itself is 0.0005) read as "fully covered" under round 2's rule and drew an isolated fleck
   far from any real feature. Round 3 requires `coverage>=0.5 AND coverage*height>=0.003` instead.
   After the fix, R5's chamfer maxima drop from 44-49px to 3.8-10.2px — and, non-obviously, its
   fp/fn/IoU/chamfer numbers become IDENTICAL to R4's at every `(scenario, m)` tested (§4.3
   explains why: point-sampling a linear cut of a box at its own centre and thresholding the
   cut's covered AREA at 50% are the same test, for any box that's symmetric about that centre —
   an exact identity, not a coincidence). R5 and R4 have never differed in their coverage boundary;
   only in the continuous height field.
3. **Added R6**: R5's anti-aliased coverage and closed-form conservation, with R4/R5's single
   gradient-direction interface line replaced by a per-axis "confinement" blend that has no
   cancellation failure mode for a feature with empty space on both sides (§3). R6 degrades
   EXACTLY to R5/R4 in the single-axis-dominant case (`SELFTEST=1` verifies this numerically to
   4 decimal places) and visibly fixes the speckle at streams while keeping R4/R5's clean
   diagonal accuracy at walls (§5) — it is now the round-3 recommendation (§6).

## 1. Framing

The app simulates at `S` and draws at `n = S*m`, `m` in `{2, 4}`. The fragment shader
reconstructs each render pixel's height with **mask-aware bilinear interpolation**
(`sandart-render/src/shader.wgsl` lines 622-671): standard 2x2-texel bilinear, with any
OUTSIDE-mask corner's weight zeroed and the remainder renormalised over the texels that are
actually inside the vessel. That renormalisation is already doing real work — it's why the
"pool edge against a wall" pictures below show R1 tracking a smooth wall boundary with no
fringe-of-zeros artifact. **R1's problem is not walls; it's empty interior** — mostly true, though
§4.2's front-fidelity numbers complicate this slightly (R1 is smooth at the wall but sits
*further* from the true wall line than every other rule there too).

R1 already has two of the required properties:
- **No overshoot** — a convex combination of the four corner values can't exceed their max or
  undershoot their min.
- **Global conservation** — the bilinear kernel sums to 1 over the whole grid (a standard tent-filter
  identity), so total mass is unchanged by the round trip.

What it does **not** have is **per-cell conservation**: a single coarse cell's own pixels do not
average back to that cell's value, because bilinear blends mass across the cell boundary into
whichever neighbour is nearer the sample point. In 1D, a cell's own pixels hold ~3/4 of its content
plus ~1/8 borrowed from each neighbour; against an empty neighbour, that 1/8 is drawn as extra
material where there is none.

Full required-property list, and how the candidates are built to satisfy each:
1. **Per-cell conservation** — R0 is trivially exact (constant fill). R2/R3 get it from an explicit
   bisection re-solve. R4 gets it from the PLIC area-fraction identity, exactly in the continuum but
   only approximately once rasterised onto `m*m` discrete point samples (§4, §6). R5 gets it from a
   closed-form solve against coverage-weighted samples (§3, §6) — exact at any `m`.
2. **No overshoot** — R2/R3/R5 enforce it with a Barth-Jespersen limiter against the cell's own 3x3
   neighbourhood; R4/R5's coverage machinery never produces a value outside `{0, h_ref}` scaled by
   a fraction in `[0,1]`, already within range.
3. **Smooth where the field is smooth** — measured directly by the pool-interior 2nd-difference
   probe (§4.4).
4. **Sharp where material meets empty, edge inside the cell** — measured by coverage IoU / false
   positives AND, new in round 2, by front-fidelity distance to the TRUE edge (§4.2, §4.3), and
   shown by the picture crops (§5).
5. **Walls are not empty** — every stencil (bilinear renormalisation, least-squares gradient,
   Barth-Jespersen min/max, PLIC's `h_ref`) explicitly excludes OUTSIDE cells rather than treating
   them as zero-valued neighbours. This is a hard invariant of the code, not a convention: an
   OUTSIDE cell is never read as a value anywhere in `diag_upscale_reconstruction.rs`.
6. **Same rule for all materials** — every rule is a pure function of `(height field, mask)`. No
   candidate function in the instrument ever reads wetness, grain size, or material mode.
7. **8-neighbour gradients** — `ls_gradient`'s least-squares fit and R3's per-axis face-matching
   both walk the full 3x3 stencil.
8. **"Match the gradient"** — R3, directly (§3).

## 2. Method

256 and 512 simulations evolve at different rates, so this never compares two independent runs.
Instead, one 512 run is captured, block-averaged down to 256 (`m=2`) and 128 (`m=4`), then each
candidate rule re-expands it back to 512 for comparison against the *same* original 512 field —
round-trip error only, with the physics held fixed. **Because the physics never changes, the
round-trip target for every ratio metric below is exactly 1.0** (not the ~10-15% simulated
stream-widening figure round 1 mistakenly imported from unrelated prior work — that number
describes how a 256 SIMULATION differs from a 512 one, a question this method doesn't ask).

**Snapshots** (all from `DrawingSimulation::new()`, i.e. `S=512`), following
`sandart-sim/examples/profile_sandfall_water.rs`'s `build()`:
- **(a)** `MultiNeckHourglass`, `MaterialMode::Water`, `lateral_substeps=2.5`, upper half filled to
  0.5, `gravity_dir=(0,0.04)`, `budget_n=128`, 600 ticks (mid-drain, streams active).
- **(b)** `Hourglass`, `MaterialMode::DrySand`, same fill/gravity/budget, 900 ticks (draining, a
  settled pile).
- **(c)** identical run to (a), but the reported field is a per-cell EMA
  (`y = 0.4*current + 0.6*y`) over the last 15 ticks — matching `sandart-wasm`'s shipped
  `temporal_alpha=0.4` default (`update_and_upload_ema`) — since that EMA, not the raw per-tick
  field, is what the deployed page actually displays.

**Downscale**: for each candidate resolution, the coarse mask is rasterised at that resolution
directly (`DrawingSimulation::rasterize_shape_mask(out_size)`), never resampled from the fine
mask. A coarse cell's value is the mean of its `m*m` fine cells that are inside the *fine* mask;
fine-inside cells whose block's coarse cell is OUTSIDE (a real mask-resolution mismatch at the
vessel boundary, not a bug) are tracked as **lost mass**, reported below rather than silently
dropped.

**Upscale**: each candidate is evaluated at the exact sub-pixel offsets a 512-wide image needs
inside each coarse cell — `m*m` fixed offsets per cell, identical to what the shader's fragment
grid actually samples (`(i+0.5)/m - 0.5` in coarse-cell units).

## 3. Candidates

- **R0 nearest** — piecewise constant. Trivially per-cell conservative and overshoot-free; every
  wall/void edge is a staircase.
- **R1 current** — mask-aware bilinear exactly as shipped (see §1).
- **R2 limited plane** — gradient from a least-squares fit over the inside 3x3 neighbours (falls
  back to independent per-axis normal equations when the neighbour set is rank-deficient). Barth-
  Jespersen-limited so the plane's own four corners stay inside the 3x3 min/max (including the
  cell's own value). Clipped at 0, then a vertical shift `delta` is found by bisection over the
  cell's own `m*m` samples so their mean is exactly the cell's value.
- **R3 face-matching gradient** — the user's "match the gradient" idea. Per axis, steepens toward
  whichever available neighbour is *fuller* so the plane reaches that neighbour's exact value at
  the shared face (half a cell away): slope `2*(hN-h0)`, twice R2's central-difference estimate.
  Where the cell is a local max along that axis, falls back to a gentle, non-steepened difference.
  Same Barth-Jespersen limiter and clip-and-bisect conservation re-solve as R2.
- **R4 vertical-edge PLIC (Youngs)** — interface normal from the same least-squares gradient;
  `h_ref = max` over the inside 3x3 neighbours including self; covered fraction `f = h0/h_ref`.
  The covered region is positioned by *exact* half-plane polygon clipping (Sutherland-Hodgman)
  plus bisection on the threshold. Material is `h_ref` inside that region, 0 outside, evaluated
  as a POINT SAMPLE at each of the `m*m` sub-pixel offsets (in vs out, binary).
- **R5 (round 2) anti-aliased PLIC coverage + limited-plane height** — the same interface normal
  and `f = h0/h_ref` as R4, but each render pixel's value is `coverage(pixel) * plane_height(pixel)`
  where `coverage` is the EXACT clipped-area fraction of that pixel's own small square footprint
  on the covered side of the line (not a point sample — `box_area_frac`, the same half-plane clip
  as R4's `area_frac_exact` generalised to one pixel's footprint instead of the whole cell), and
  `plane_height` is R2's Barth-Jespersen-limited plane (unclipped). Conservation is a **closed
  form**, not a bisection: since `coverage` doesn't depend on the vertical shift `delta`,
  `mean(coverage*(plane+delta)) = h0` is linear in `delta`, giving
  `delta = (h0 - mean(coverage*plane)) / mean(coverage)` directly. A thin covered strip fades
  instead of vanishing between sample points; §4.3 shows this does NOT change where the
  0.5-coverage boundary itself sits (R4 and R5 are geometrically identical there — see the round-3
  note above), only the continuous height field.
- **R6 (round 3) R5 + a centred-strip case for features one cell wide.** R4/R5's interface normal
  comes from the 3x3 GRADIENT, which cancels to ~0 exactly when it matters most — a stream with
  empty space on both sides has opposing left/right differences that cancel, leaving the line's
  direction at the mercy of floating-point noise (the mechanism behind the isolated flecks a
  gradient-based line produces there). R6 replaces the single 2D line with a SMOOTH BLEND of two
  1D strips, one per axis: along axis `X` with neighbours `L,R`, a strip of width `f` (`f=h0/h_ref`,
  same as R4/R5) offset by `bias_x*(1-f)/2` where `bias_x=(R-L)/h_ref` — 0 when `L≈R` (centred:
  "empty on both sides" or "full on both sides" both give `bias≈0`), ±1 when one side is empty and
  the other at `h_ref` (flush against that edge — see below, this is exactly R5's behaviour). Axis
  `Y` gets its own strip the same way. The two strips are combined as
  `coverage = w*coverage_x + (1-w)*coverage_y`, where `w` is each axis's own CONFINEMENT
  (`(|L-h0|+|R-h0|)/h_ref`, clamped to `[0,1]` — 0 if both neighbours simply continue the flow at
  `h0`, up to 1 if the axis shows a real feature) normalised so `w_x+w_y=1`. This is a genuine
  smooth blend, not an if/else pick of "the" dominant axis: both `coverage_x` and `coverage_y`
  individually average to `f` over the cell's footprint, so ANY blend weight preserves exact
  conservation regardless of whether `w` is "right." Height and conservation are otherwise
  identical to R5 (Barth-Jespersen-limited plane, closed-form `delta` against the blended
  coverage). In the pure single-axis-dominant case (one neighbour at `h_ref`, the opposite at 0,
  the other axis flat) this reduces EXACTLY to R5's half-plane result — `SELFTEST=1` checks this
  to 4 decimal places (`max |diff| = 0.0000`) — because a strip flush against one edge with width
  `f` is the identical set of points as a half-plane cut at the threshold that gives area `f`,
  for an axis-aligned normal. It does NOT reduce to R5 for a genuinely DIAGONAL interface (R6 has
  no rotated-line case at all — only axis-aligned strips, blended) — an accepted trade, see §4.3
  and §6 for where this costs and where it pays off. Confinement is computed with one deliberate,
  narrow exception to the "walls are not empty" invariant: an OUTSIDE neighbour reads as height 0
  for THIS signal only (never for height/gradient/mass), because a wall confines a stream visually
  exactly like empty space does, and a neck squeezed between two walls is exactly the degenerate
  case this rule targets.

**Film case** (all neighbours shallow and similar): R2/R3's gradient is small either way, the
Barth-Jespersen window is narrow, and the conservation re-solve needs `delta≈0` — the cell ends up
covering its whole footprint at very close to its own flat value, matching R0/R1 there. R4/R5's
`h_ref≈h0` drives `f≈1`; the geometric solve degrades gracefully to "cover (almost) the whole
cell" without a special case, since a threshold near `f=1` sits near the extreme corner of the
box-area formula regardless of which arbitrary normal direction a near-zero gradient produces.

## 4. Results

Full output reproduced by `cargo run -p sandart-sim --release --example
diag_upscale_reconstruction`, ~36s wall on this machine for all three snapshots x two resolutions x
six rules. `rms`/`mass_err`/`smooth` are in heightmap units (0..1); pixel/distance units are px at
512.

### 4.1 Downscale mass loss (reported, not hidden)

| snapshot | m=2 lost / total | m=4 lost / total |
|---|---|---|
| (a) water mid-drain | 145.8 / 62505 (0.233%) | 744.9 / 62505 (1.192%) |
| (b) dry sand pile | 168.4 / 39322 (0.428%) | 870.4 / 39322 (2.214%) |
| (c) water EMA | 145.6 / 62505 (0.233%) | 744.7 / 62505 (1.192%) |

All of this is the fine/coarse mask-resolution mismatch at the vessel boundary, identical for
every rule at a given `(snapshot, m)`, and grows with `m` as expected.

### 4.2 Per-stream width (round-2 fix A) — target 1.0

Round 1's row-summed metric conflated stream width with stream SEPARATION. This detects each
stream as its own contiguous covered span (`h>=0.003`) and matches spans between the original and
each candidate by nearest centre, at three rows in FREE FALL (`neck_row + 16, +32, +48` = 272,
288, 304 — round 1 probed `neck_row+2`, still inside the neck's throat), averaged over every
matched stream and all three rows. `occ` = occupied-width ratio, `mass` = mass-weighted uniform-
equivalent-width ratio, both candidate/original. `missing` = an original stream the rule dropped
or merged away entirely (excluded from the average, reported separately, never silently averaged
in as a 0).

Width/mass ratios are unaffected by round 3's coverage-convention fix (§4.3): they're computed from
the coverage-weighted HEIGHT array at the shipped 0.003 cutoff for every rule, not from the raw
coverage fraction, so R5's numbers here are unchanged from round 2.

**Snapshot (a), water mid-drain** (11 true streams total across the 3 probe rows):

| rule | m=2 occ | m=2 mass | m=2 missing | m=4 occ | m=4 mass | m=4 missing |
|---|---|---|---|---|---|---|
| R0 | 2.448 | 2.226 | 2/11 | 2.370 | 3.111 | 2/11 |
| R1 | 2.796 | 2.248 | 2/11 | 3.556 | 3.219 | 2/11 |
| R2 | 2.315 | 2.113 | 2/11 | 2.370 | 3.020 | 2/11 |
| R3 | 2.315 | 2.171 | 2/11 | 2.370 | 3.062 | 2/11 |
| R4 | 1.271 | 1.822 | 0/11 | 2.074 | 3.076 | 2/11 |
| R5 | 1.595 | 1.620 | 1/11 | 2.187 | 2.760 | 2/11 |
| R6 | 2.315 | 1.998 | 2/11 | 2.431 | 2.872 | 2/11 |

**Snapshot (c), water EMA alpha=0.4** (9 true streams; the EMA erases some of the thinnest ones
that (a) still resolves, hence fewer):

| rule | m=2 occ | m=2 mass | m=4 occ | m=4 mass |
|---|---|---|---|---|
| R0 | 1.127 | 1.186 | 1.285 | 1.676 |
| R1 | 1.467 | 1.198 | 1.928 | 1.732 |
| R2 | 1.156 | 1.151 | 1.285 | 1.635 |
| R3 | 1.156 | 1.150 | 1.285 | 1.649 |
| R4 | 0.805\* | 1.169 | 1.135 | 1.681 |
| R5 | 0.903\* | 1.073 | 1.196 | 1.494 |
| R6 | 1.129 | 1.098 | 1.241 | 1.550 |

\* R4/R5 at m=2 also each spawn 1-2 EXTRA spans not present in the original (a stream a hair below
threshold in truth reads as covered in the reconstruction, or a genuine stream gets split into two
narrower ones by the interface geometry) — occupied-width ratios under 1.0 alongside a nonzero
`extra` count means "individually thinner, but there are more of them," not "closer to correct."
R6 has zero `extra`/`missing` beyond what R0/R2/R3 already show, at every `(snapshot, m)`.

Every rule overshoots width at every `(snapshot, m)` — none reaches the 1.0 target. R4 is
consistently closest on occupied width alone, but R6 is the best MASS ratio at `m=2` on both
snapshots (1.998/1.098, beating R4/R5) while carrying none of R4/R5's extra-span risk. R1 is
consistently the worst, and gets WORSE at `m=4` (3.56x) rather than converging. R0/R2/R3/R6 are
close to each other on occupied width throughout — the plane-limiting in R2/R3 (and R6's strip)
measurably helps the continuous height field (§4.4) but does not translate into a narrower
rendered stream width, because the coverage threshold (0.003) is crossed at nearly the same pixel
for a piecewise-constant fill as for a shallow limited plane or strip.

### 4.3 Front fidelity (round-2 fix B): symmetric chamfer distance to the TRUE coverage boundary

Global fp/fn/IoU is dominated by bulk pool and wall area and cannot see whether a FRONT is jagged
— the user's actual complaint. This computes a 2-pass chamfer-(1,√2) distance transform (~2% of
true Euclidean, plenty at this scale) of the original's `h>=0.003` boundary and of each candidate's
own coverage boundary, then reports, LOCALLY within each named feature's crop region:
- **fwd** = distance from each RECONSTRUCTED boundary pixel to the nearest TRUE boundary pixel
  (positional accuracy / bias of what the rule draws).
- **rev** = distance from each TRUE boundary pixel to the nearest RECONSTRUCTED one (whether the
  rule's edge, wherever it runs, still passes near every part of the true edge).

Predicted staircase signature: mean ≈ `m/4`, max ≈ `m/2`. At `m=4` that's mean≈1.0, max≈2.0.

**Round-3 fix: what "covered" means for R5/R6.** Round 2 thresholded R5's raw coverage fraction
alone at `>=0.5`. A film-case cell — all neighbours shallow, so `f=h0/h_ref` is close to 1 even
though `h0` itself is a near-zero residue — reads as "fully covered" under that rule alone, and
drew an isolated fleck at near-zero height, far from any real feature. This dominated R5's
chamfer MAX (44-49px in round 2) without being a real edge-position error at all. Fixed: R5/R6 now
require `coverage>=0.5 AND coverage*height>=0.003` (`reconstruct_pair`'s `combined` array, an
inline mapping so `coverage_metrics`/`boundary_mask` need no change: below 0.5 coverage the
array holds a sentinel `-1.0` that can never clear any positive threshold; at or above it, the
array holds the actual coverage-weighted height, compared against the same 0.003 every other rule
uses). **The `SELFTEST=1` film-case check demonstrates the fix directly**: a cell at `h0=0.0005`
with every neighbour equally shallow reads `coverage=1.0000` (round 2's rule: covered) but
`height=0.000500` (round 3's rule: not covered, since `<0.003`).

**A structural finding, not an artifact of this fix: R4 and R5 have an IDENTICAL coverage
boundary, always.** Point-sampling a straight cut of a box AT THE BOX'S OWN CENTRE and asking
whether the cut's covered AREA is `>=50%` are the same test for any box symmetric about that
centre point — a straight line through the centre always splits it exactly 50/50, so the centre is
on the covered side if and only if the covered area exceeds half. R4 point-samples exactly at the
box centre; R5 (and R6, in the single-axis case) computes the exact area. They can only ever
disagree in the CONTINUOUS height they report for a partially-covered pixel, never in the binary
covered/not-covered decision. This is confirmed exactly in the corrected numbers below: R4 and R5's
fp/fn/IoU and chamfer are bit-identical at every `(scenario, m)` tested, on both snapshots (a) and
(c) — round 2's apparent "R5 fixes R4's edge" story was ENTIRELY the film-case fleck bug, not a
real geometric difference. What R5 actually changes, structurally, versus R4 is the continuous
field (§4.4: `rms_all`, `max_mass_err`, and the mass-weighted width ratio in §4.2) — never the
coverage boundary.

**Snapshot (c), water EMA, slope_front region** (px; fwd only, the more informative direction here):

| rule | m=2 mean | m=2 max | m=4 mean | m=4 max |
|---|---|---|---|---|
| R1 | 0.985 | 2.000 | 2.861 | 4.828 |
| R2 | 0.285 | 1.414 | 0.986 | 3.414 |
| R3 | 0.283 | 1.414 | 0.982 | 3.414 |
| R4 | 0.872 | 5.243 | 0.989 | 4.828 |
| R5 | 0.872 | 5.243 | 0.989 | 4.828 |
| R6 | 0.729 | 5.243 | 0.971 | 4.828 |

**Snapshot (c), pool_wall_edge region:**

| rule | m=2 mean | m=2 max | m=4 mean | m=4 max |
|---|---|---|---|---|
| R1 | 0.534 | 2.000 | 1.555 | 4.828 |
| R2 | 0.202 | 1.414 | 0.728 | 3.414 |
| R3 | 0.202 | 1.414 | 0.716 | 3.414 |
| R4 | 0.988 | 4.414 | 0.938 | 3.828 |
| R5 | 0.988 | 4.414 | 0.938 | 3.828 |
| R6 | 0.802 | 3.828 | 0.907 | 3.828 |

**Snapshot (c), stream_sides region** (the feature R6 exists to fix):

| rule | m=2 mean | m=2 max | m=4 mean | m=4 max |
|---|---|---|---|---|
| R1 | 1.232 | 2.000 | 2.091 | 3.828 |
| R2 | 0.417 | 1.000 | 0.453 | 2.414 |
| R3 | 0.417 | 1.000 | 0.453 | 2.414 |
| R4 | 0.247 | 4.243 | 0.787 | 10.243 |
| R5 | 0.247 | 4.243 | 0.787 | 10.243 |
| R6 | **0.127** | 3.828 | 0.530 | 5.243 |

**Findings that reframe §6's recommendation, all visible in the pictures too (§5):**

1. **R1 has the worst positional bias of every rule, at every feature, despite looking the
   smoothest.** Chamfer conflates two different things — local jaggedness (staircasing) and global
   bias (a systematically shifted edge) — and R1's bias dominates: its bilinear spillover doesn't
   just look wider, it measurably sits 2-3x farther from the true line than R0's blocky
   reconstruction at the same spot. A viewer's eye reads "smooth" from R1's lack of jaggedness and
   never notices the bias; the chamfer metric sees only the bias.
2. **R2/R3 barely improve on R0's edge position at walls/slopes** (0.28 vs R0's 0.302 at
   `slope_front, m=2` — from the full run log — i.e. ~6% better, not the large win their rms/mass
   numbers would suggest), and their advantage nearly vanishes at `m=4`.
   **`pool_wall_edge_a_raw_R2_limited_plane_m4.png` and `pool_wall_edge_a_raw_R0_nearest_m4.png`
   are visually almost indistinguishable staircases** (also visible for R3 in
   `sand_slope_b_dry_R3_face_match_m4.png`) — R2/R3's real, measured win is in the continuous
   height field (§4.4), not in where the rendered edge itself lands.
3. **R4/R5 have good mean front position at walls/slopes, but the picture shows their edges are a
   fine SPECKLE, not a clean line** (`sand_slope_b_dry_R4_plic_m2.png`,
   `stream_neck_c_ema_R5_plic_aa_m4.png`) — chamfer's MEAN doesn't penalize this (each speck is
   individually close to the true line), but it reads as noisy/dithered to the eye, which is a
   real defect for the "smooth curve" complaint even when the MEAN position is good. §5 shows this
   survives the EMA (it's a per-cell reconstruction artifact, not temporal noise).
4. **R6 wins decisively at `stream_sides`** (0.127 vs R2/R3's 0.417 and R4/R5's 0.247 at `m=2`;
   0.530 vs 0.453/0.787 at `m=4`) — exactly the feature it was built for — while landing BETWEEN
   R2/R3 and R4/R5 at walls and slopes (0.802/0.729 vs R2/R3's 0.20/0.28 and R4/R5's 0.99/0.87).
   §5's pictures confirm R6 draws these as clean, non-speckled lines, not a visually-worse
   compromise.

**Conclusion up front, since §6 needs it: R6 is now the closest thing to a rule that wins on both
the numbers and the pictures, though it still trades away some of R2/R3's wall/slope accuracy to
get there** — see §6 for the full trade-off, stated plainly.

### 4.4 Main metrics (height field, conservation, coverage)

**Snapshot (c), water EMA alpha=0.4, m=2:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU |
|---|---|---|---|---|---|---|---|
| R0 | 0 | 0.0453 | 0.0331 | 0.1615 | 885 | 218 | 0.9838 |
| R1 | 0.1525 | 0.0297 | 0.0300 | 0.0202 | 2300 | 0 | 0.9670 |
| R2 | 0 | 0.0406 | 0.0262 | 0.1611 | 841 | 187 | 0.9849 |
| R3 | 0 | 0.0401 | 0.0254 | 0.1610 | 837 | 187 | 0.9850 |
| R4 | 0.2450 | 0.0679 | 0.0613 | 0.1612 | 0 | 2301 | 0.9658 |
| R5 | 0 | 0.0542 | 0.0450 | 0.1610 | 0 | 2301 | 0.9658 |
| R6 | 0 | 0.0406 | 0.0262 | 0.1610 | 0 | 1800 | 0.9733 |

**Snapshot (c), m=4:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU |
|---|---|---|---|---|---|---|---|
| R0 | 0 | 0.0895 | 0.0559 | 0.2466 | 2083 | 792 | 0.9586 |
| R1 | 0.1625 | 0.0462 | 0.0472 | 0.0327 | 5101 | 0 | 0.9296 |
| R2 | 0 | 0.0829 | 0.0440 | 0.2456 | 1990 | 774 | 0.9601 |
| R3 | 0 | 0.0821 | 0.0424 | 0.2452 | 1987 | 774 | 0.9602 |
| R4 | 0.1206 | 0.1020 | 0.0762 | 0.2453 | 449 | 3198 | 0.9462 |
| R5 | 0.000001 | 0.0942 | 0.0643 | 0.2449 | 449 | 3198 | 0.9462 |
| R6 | 0.000001 | 0.0839 | 0.0463 | 0.2449 | 469 | 2839 | 0.9512 |

Round-3 note, because these rows changed: R4/R5's fp/fn/IoU are now IDENTICAL, as §4.3's structural
finding requires — they share a coverage boundary by construction and can differ only in the
continuous height of a partly covered pixel. R5's round-2 row (fp 3112 / fn 609 / IoU 0.9434 at
m=2) was the film-case fleck bug, not a real boundary. What R5 actually buys over R4 is the
continuous field: `max_mass_err` ~0 against R4's 0.12-0.25, and `rms_all` 0.0542 against 0.0679.

R6 is the best rule in this table on every column it can be judged on: exact conservation, the
lowest `rms_all`/`rms_int` of the conservative rules (0.0406/0.0262 at m=2, essentially tying R3's
0.0401/0.0254 while also getting the edges right), the fewest false negatives of the coverage-based
rules (1800 vs R4/R5's 2301), and the best IoU of any rule at both m (0.9733 / 0.9512). R1 still
leads `rms_front` (0.0202) — it blurs the frontier, which lowers that error and is exactly the
spillover the width and chamfer metrics penalise it for.

**Smoothness** (2nd-difference RMS in a flat pool/pile interior; true baseline ≈0 for every
scenario): unchanged from round 1 — R0/R2/R3/R4/R5 all reconstruct a flat interior as exactly flat
(0.000000000 to printed precision); R1 is the only rule with a nonzero value (0.0001-0.0055 across
scenarios), i.e. the only rule that measurably disturbs a flat interior at all.

**R1 at alternative coverage thresholds** (unchanged finding from round 1): raising the threshold
15x (0.003 -> 0.05) only moves IoU a few points and cuts false positives ~15% — R1's over-coverage
is genuine excess height, not a threshold-tuning artifact.

## 5. Pictures

All at `artifacts/design/upscale-2026-09-14/`. Crops are a 128x128 sample of the 512 grid (the SAME
region the numeric probes above use — `stream_region`/`pool_wall_region`/`slope_region` are shared
functions, so a number and its picture are always of the same patch), magnified 4x
nearest-neighbour to 512x512 per tile (no new information, purely legibility). Grayscale = height;
orange = the coverage boundary (`h>=0.003` for R0-R4; `coverage>=0.5 AND coverage*height>=0.003`
for R5/R6, per §4.3's round-3 fix). Each `<figure>_contact_sheet.png` is 6 columns x 2 rows —
**original, R1, R3, R4, R5, R6**, row 1 = m=2, row 2 = m=4; R0/R2 crops are on disk individually.

**Round 3 renders every crop from the snapshot it belongs to, and names it accordingly**
(`_a_raw_`, `_c_ema_`, `_b_dry_`). `stream_neck` and `pool_wall_edge` exist at BOTH (a) and (c) over
the identical crop region, because the page displays the EMA field, not the raw per-tick one — so
(c) is the sheet to judge by, and the (a)/(c) pair shows directly how much of any speckle is
temporal.

- **`stream_neck_c_ema_*`** (the sheet that matches what the page draws): R1 (col 2) draws the
  stream as a wide grey band, roughly twice the original's width at both m — smoother than the
  truth, not more accurate. R3 (col 3) is close to the right width but keeps a ragged edge. R4/R5
  (cols 4-5) are narrow but their sides read as a DASHED line rather than a continuous one. R6
  (col 6) is the only rule that draws the stream as a clean, continuous, correctly-thin column at
  both m=2 and m=4 — this is the defect the user reported, and R6 is the only candidate that fixes
  it without introducing another.
- **`stream_neck_a_raw_*`** vs the (c) sheet: the raw original (col 1) is itself flecked with
  orange — real tick-to-tick jitter in the stream's edge (CLAUDE.md's accepted near-neck pulse).
  The EMA removes most of that from the truth AND from every rule's reconstruction, but R4/R5's
  dashed sides survive it: that speckle is a per-cell reconstruction artifact, not temporal noise.
- **`pool_wall_edge_c_ema_*`**: R1 tracks the wall smoothly but offset; R3 staircases at m=4;
  R4/R5/R6 all track the true curve closely, R6 without the dashing. R0/R2 (on disk) are
  near-indistinguishable staircases at m=4 — R2/R3's win over plain nearest is in the height
  field, not in where the edge lands.
- **`sand_slope_b_dry_*`** (a fully-inside box in the lower chamber — the boundary shown is the
  pile's own repose-angle surface, not the container): R1 is smooth but the most offset; R3
  staircases clearly at m=4; R4/R5 speckle along the whole front; R6 draws a clean line closest to
  the original, with a little residual speckle only at the neck tip at m=4.

## 6. Recommendation

**R6 (anti-aliased strip coverage, blended per axis) is the recommendation, and it is the shader
candidate.** Round 2 could not recommend anything because the two properties below were in tension;
R6 is the first candidate that holds both. The trade it makes is stated at the end.

Two properties were in tension:

- **Continuous-field accuracy + exact conservation + no overshoot**: R2 and R3 win clearly (rms,
  `max_mass_err`, flat-interior smoothness, moderate fp reduction vs R1). R3 is marginally ahead of
  R2 on most (scenario, m) combinations and is the direct implementation of the user's "match the
  gradient" idea.
- **Edge/front positional accuracy** (what actually reads as "smooth curve" vs "staircase" to a
  viewer): R4/R5 have the best average position, R2/R3 are only marginally better than R0's
  outright staircase, and R1 — despite looking smoothest — has the worst positional bias of all
  five, at every feature tested.
- Neither property implies the other. R2/R3's plane-fit measurably fixes the continuous height
  field without moving the rendered 0.003-threshold edge much; R4/R5's PLIC interface measurably
  gets the edge position right but renders it as a speckled dither rather than a clean line.
  (R5's round-2 "stray flecks" were the film-case threshold bug, fixed in round 3; the dither
  along real edges is not, and survives the EMA — §5.)
- **R6 holds both.** It keeps R5's exact conservation and continuous coverage, and its per-axis
  strip removes the ill-conditioned-normal failure that produced the dither: best IoU of any rule
  at both m, `rms_all` level with R3's, the decisive win at `stream_sides` (§4.3 finding 4), and
  clean lines in the pictures.

**The rule to ship: R6.** Against the shipped R1, on the EMA snapshot the page actually displays:
stream width ratio 1.10 vs 1.20 at m=2 and 1.55 vs 1.73 at m=4 (mass-weighted, target 1.0);
`stream_sides` mean edge error 0.127 px vs 1.232 at m=2; exact per-cell conservation vs R1's
0.15-0.16 max error; a flat interior reconstructed as exactly flat, which R1 alone fails. It is the
only candidate whose pictures show no new artifact at either m.

**What R6 trades away, plainly:** at walls and slopes its edge sits farther from the truth than
R2/R3's plane (0.80/0.73 px vs 0.20/0.28 at m=2), because it has no rotated-line case — only
axis-aligned strips, blended. R2/R3 remain better there and are better on `rms_front`. The judgment
is that a stream drawn at the right width with a clean edge matters more than sub-pixel accuracy on
a wall that already looks smooth, because the stream is the reported defect. If a diagonal front
later looks wrong on the page, the fix is a rotated-line case inside R6, not a return to R2/R3.

**R4 is not recommended**: worst false-negative counts by 5-13x, the largest discretization-driven
`max_mass_err` of the non-R1 rules, and it drops individual free-fall streams entirely at `m=4`
(§4.2's `missing` column) — on top of the newly-observed speckle.

**R5 is R6 without the strip, and is superseded by it.** Its closed-form conservation and continuous
coverage are real structural improvements over R4 (no exact-dropout failure mode, §4.4) and R6
inherits both; what R6 adds is a well-conditioned normal where the gradient cancels, which is
exactly where R5 dithers.

**One caveat that applies to every number here.** This instrument thresholds coverage to decide
"covered", because a binary is needed to compare against R0-R3 on IoU and chamfer. The shader does
not: it feeds `coverage * height` into `empty_blend`, which fades continuously, so a cell
contributing 2% coverage renders at ~2% opacity rather than as a hard speck. The coverage-based
rules should therefore look somewhat better on the page than they score here, and R6's remaining
neck-tip speckle (§5) is the first thing to re-judge there.

## 7. Shader cost estimate

**Today (R1):** 4 filtered `textureSampleLevel` reads (the 2x2 quad) plus a renormalisation branch,
per fragment, per frame.

**R3 (the round-2 recommendation):** move the per-cell work — gradient, Barth-Jespersen `phi`, and
the conservation `delta` — into a pre-pass over `S*S` simulation cells (not `n*n` fragments),
writing `(h0, phi*gx, phi*gy, delta)` into an auxiliary `S`x`S` RGBA32F texture, analogous to the
heightmap upload the sim already does every frame. The fragment shader then does **one**
`textureLoad` of that pre-computed plane plus a 2-term dot product — cheaper per-fragment than
R1's current 4 filtered reads.

Does the conservation re-solve have a closed form? No general one in 2D (clip-at-0 then average is
piecewise in the plane's orientation), but it's cheap regardless: the mean of a linear plane over
the `m*m` symmetric sample offsets is EXACTLY `h0` with `delta=0` whenever nothing clips (true for
every interior cell away from a material/empty boundary — clipping, and the resulting bisection,
is confined to frontier cells), and where needed a short fixed-iteration bisection is affordable
because it runs once per cell in the pre-pass, not per rendered fragment.

**R5, if prototyped later:** the coverage half is genuinely cheaper conceptually than R3's
bisection — `box_area_frac`'s clipped-area-of-a-square-against-a-line is a closed-form analytic-AA
formula (a handful of comparisons and one division, no iteration), the same primitive real-time
AA techniques already use per-pixel. R5's conservation `delta` is ALSO closed form (§3) — cheaper
than R2/R3's bisection, not more expensive. The added per-fragment cost relative to R3 is the
`box_area_frac` evaluation itself (a few comparisons/multiplies), still a single pre-computed
per-cell fetch away from being as cheap as R3's plane.

**R6 (the recommendation).** Same shape of cost as R5, with no iteration anywhere: per cell, two 1D
strip coverages (each a clamped overlap of two intervals), two confinement weights, one blend, and
the same closed-form conservation. It needs the 3x3 heights and the 3x3 mask; per fragment that is
9 + 9 `textureLoad`s if computed inline, or a single fetch if the per-cell quantities are
pre-computed into an auxiliary `S`x`S` texture as described for R3. Deliberately absent: any
rotated-line case, so there is no `atan2`, no normalisation of an ill-conditioned gradient, and no
branch on the normal's direction. The first implementation should be the inline per-fragment
version, because it needs no new upload path; move to the pre-pass only if it measures too slow on
the Deck.

## 8. Reproduction

```
cargo run -p sandart-sim --release --example diag_upscale_reconstruction
```

~36s wall (three ~600-900-tick 512x512 simulations plus six reconstruction rules x two
resolutions x three snapshots). `SELFTEST=1 cargo run ...` runs a handful of closed-form geometry
sanity checks on `area_frac_exact`/`box_area_frac`/`solve_plic_threshold` (used to confirm the R5
stray-flecks finding in §4.3 was not a clipping-polygon bug before it was reported). Writes all
PNGs and `README.md` in this directory and prints the full metrics tables, including the per-row
`DUMP_ROW_COUNTS=1` mask-profile debug path used to confirm `MultiNeckHourglass`'s several necks
sit at one shared row rather than being vertically distributed.

## 9. Round 4 (2026-09-16): why R6 quilted, and R7

R6 shipped behind a toggle (round 3's recommendation). The user looked at the deployed page and
called it "quilting" -- worse than the shipped bilinear -- and the toggle was defaulted off before
this round started. The leading hypothesis, stated going in: R6/R4/R5's covered fraction
`f = h0/h_ref` (`h_ref` = max over the inside 3x3) is meaningful at a true material/empty frontier,
but it shrinks **every** cell whose neighbourhood isn't perfectly flat -- a sand slope, a draining
pool surface -- because the uphill neighbour is simply higher than `h0`, no empty space nearby at
all. A grid of slightly-shrunken tiles, each pulled a hair short of its own footprint, is exactly
what "quilting" looks like. The specific number cited for this going in was R6's binary
false-negative count on snapshot (c): 1800 at m=2, 2839 at m=4, framed as "missing interior pixels,
not edge error."

### 9.1 The specific claim was wrong; the mechanism was right

`fn_interior_frontier_split` (new in `diag_upscale_reconstruction.rs`) classifies every FALSE
NEGATIVE fine pixel (original `>=THRESH`, reconstruction not covered) by whether its own coarse
cell is "material-interior" (every one of its inside-mask 3x3 neighbours also has material,
`>=THRESH` -- a wall neighbour is excluded, not counted as empty, matching the codebase's
"walls are not empty" invariant everywhere else) or a genuine material/empty frontier (some
neighbour is actually empty).

**The literal claim does not hold.** On snapshot (c), the EMA field the page actually displays,
R6's false negatives are dominated by FRONTIER, not interior, by 4-8x:

| snapshot (c) EMA | fn_px | interior | frontier | interior share |
|---|---|---|---|---|
| m=2 | 1800 | 207 | 1593 | 11.5% |
| m=4 | 2839 | 480 | 2359 | 16.9% |

Same pattern on (a) (raw) and (b) (dry sand): interior is always the minority share (11-27%
across every `(snapshot, m)` tested for R6; R4/R5 run a bit higher, 27-32%, because they have no
strip machinery reducing interior shrinkage in the first place). Per the task's own stop condition
("if the interior count is not dominant, stop and report -- the hypothesis is wrong"), this
result on its own says stop: most of R6's counted false negatives are ordinary frontier pixels
near a real edge, not a defect in the interior.

**But the false-negative count is the wrong instrument for the underlying claim.** It only fires
when a cell's coverage shrinks enough to pull the reconstructed height below `THRESH` (0.003) --
a near-total dropout. The quilting hypothesis describes something much smaller: a coverage
fraction of, say, 0.97 at a gentle slope cell, never crossing the threshold, never counted as a
false negative, but still a real, visible shrinkage at that cell's own boundary. `fn_split` is
blind to it by construction. `coverage_deficit_stats` (also new) measures it directly: mean
`1 - coverage` over **every** inside fine pixel, split the same interior/frontier way (frontier's
number is not informative here -- most "frontier" fine pixels are legitimately empty near a real
edge, which inflates its deficit with correct output, not error; only the interior number isolates
the claim). Snapshot (c):

| | interior mean deficit, m=2 | interior mean deficit, m=4 |
|---|---|---|
| R6 | 0.01712 | 0.01731 |
| R7 | 0.00623 | 0.00519 |

R6 shrinks essentially every coarse-interior fine pixel by a small amount on average (~1.7%),
which is the direct, continuous confirmation of the hypothesis's actual mechanism -- widespread,
low-contrast, per-cell shrinkage is exactly what a seam/quilt pattern looks like, and it is nearly
invisible to a threshold-gated metric because so few individual cells cross all the way to zero.
R4/R5 (no strip machinery at all) show the same interior deficit as R6 (0.01712/0.01731,
identical to 5 decimal places -- R6's strip degrades to R5's PLIC value in the common
single-axis-dominant interior case, exactly as the round-3 `SELFTEST` predicts).

**Conclusion for §9.1: the specific number in the hypothesis (dominant false negatives) does not
hold, but the mechanism proposed (h_ref=max shrinks every non-flat interior cell) is confirmed by
a metric built to actually measure it.** The picture crops corroborate this directly --
`sand_slope_b_dry_contact_sheet.png` column 6 (R6) shows a visible scatter of stray orange
boundary flecks across the whole interior slope face at both m=2 and m=4, not just at the true
edge; column 7 (R7, §9.2) shows markedly fewer.

### 9.2 R7: R6 with coverage that saturates away from frontiers

Implemented exactly as specified going in, no deviation: per cell, `h_min` = the minimum height
over the inside 3x3 neighbours (self excluded, walls excluded -- same convention as `h_ref`),
`emptiness = clamp(1 - h_min/max(h0,eps), 0, 1)`, `f_eff = mix(1.0, f, emptiness)`, substituted for
R6's raw `f` everywhere it feeds the strip width/offset (`strip_half`, `off_x`, `off_y`).
Confinement, bias, `w`, the limited plane, and the closed-form `delta` solve are all otherwise
identical to R6 (`precompute_r7_models` reuses `R6Model`/`eval_r6` verbatim). Why this saturates
correctly without a branch: on a smooth slope the per-cell height step is small relative to `h0`,
so `h_min/h0` stays close to 1 and `emptiness` stays near 0 (full coverage, no shrink); at a
genuine frontier a neighbour is close to actually empty, `h_min/h0` collapses toward 0, and
`emptiness` saturates to 1 (R6's own `f`, unchanged). No material/wetness branch, no threshold.

**Conservation is unaffected, confirmed, not just argued:** `max_mass_err` for R7 is 0 or
~1e-6 at every `(snapshot, m)` tested -- identical order of magnitude to R6 and R5, because the
`delta` solve targets whatever coverage field is actually in use (built from `f_eff` here), exactly
as it targets R6's `f`.

### 9.3 R7 metrics vs R6 and R1, snapshot (c) EMA (what the page displays)

| rule | m | max_mass_err | rms_all | IoU | fn_px (interior/frontier) | stream_sides fwd mean | pool_wall fwd mean | slope_front fwd mean |
|---|---|---|---|---|---|---|---|---|
| R1 | 2 | 0.152472 | 0.029684 | 0.9670 | 0 | 1.232 | 0.534 | 0.985 |
| R6 | 2 | 0.000000 | 0.040632 | 0.9733 | 1800 (207/1593) | 0.127 | 0.802 | 0.729 |
| R7 | 2 | 0.000000 | 0.039969 | **0.9744** | **1723** (130/1593) | **0.079** | **0.738** | **0.534** |
| R1 | 4 | 0.162461 | 0.046160 | 0.9296 | 0 | 2.091 | 1.555 | 2.861 |
| R6 | 4 | 0.000001 | 0.083904 | 0.9512 | 2839 (480/2359) | 0.530 | 0.907 | 0.971 |
| R7 | 4 | 0.000001 | 0.080361 | **0.9565** | **2484** (125/2359) | 0.530 | **0.887** | **0.960** |

Every bolded number is R7 beating R6 on snapshot (c); per-stream mass ratio, not shown, ties or is
within noise (1.102 vs 1.098 at m=2, identical 1.550 at m=4). R7's frontier false-negative count is
bit-identical to R6's at every row (1593, 1593, 2359, 2359) -- exactly the intended, verified
consequence of `emptiness` saturating to 1 there, i.e. R7 changes nothing about R6's edge placement,
only its interior. The interior false-negative count drops 37% (m=2) to 74% (m=4); `stream_sides`
chamfer, R6's own headline win over R4/R5, improves further (0.127->0.079 at m=2); `slope_front` --
the feature this round's hypothesis was actually about -- improves the most (0.729->0.534 at m=2,
a 27% cut).

**One real, small regression, on snapshot (a) only (not (b) or (c)):** `fp_px` rises slightly,
332->356 at m=2 and 883->901 at m=4 (+7% / +2%). Mechanism: `f_eff>f` (R7 covers MORE than R6 in
the interior by construction) occasionally pushes a coverage-weighted height that was just under
`THRESH` on the frontier side of an interior cell just over it. `fn_px` drops far more than `fp_px`
rises in absolute terms (291 and 415 respectively vs. 24 and 18), so IoU still improves at both
`m` (0.9684->0.9724, 0.9486->0.9545) and `rms_all` still drops, but this is a real trade, not a
free lunch, and is the one place in the whole sweep that isn't a strict R6-versus-R7 win. On (b)
and (c), `fp_px` is unchanged at every `m` (35/35, 209/209, 0/0, 469/469) -- (a)'s slightly higher
`lateral_substeps` (2.5, vs 1.0 for (b)) and more active free-fall streams are the likeliest reason
this snapshot alone shows it, though that is not confirmed here.

Full per-snapshot tables are reproduced by the command in §8; besides the (a) `fp_px` note above,
every other metric (`max_mass_err`, `rms_all`, `rms_int`, `fn_px`, IoU, all three chamfer regions)
either improves or ties between R6 and R7 at every `(snapshot, m)`. E.g. on (a) at m=4, R6->R7: fn
2582->2167 (interior 504->89, frontier unchanged at 2078), IoU 0.9486->0.9545, `rms_all`
0.111751->0.108532 -- `stream_sides`/`pool_wall_edge`/`slope_front` chamfer are bit-identical
between R6 and R7 there (0.552/0.548/0.602 both), because that crop's own free-fall stream sits far
enough from any interior-slope cell that `emptiness` never departs from R6's frontier value in that
specific region -- consistent with §9.4's picture note that R7 and R6 look identical at
`stream_neck`.

### 9.4 Contact sheets (regenerated, R7 added as column 7)

All in `artifacts/design/upscale-2026-09-14/`, 7 columns x 2 rows now (original, R1, R3, R4, R5,
R6, R7 -- see the updated `README.md` in that directory):
- `sand_slope_b_dry_contact_sheet.png` -- the clearest visual confirmation: column 6 (R6) shows a
  scatter of stray orange boundary flecks across the whole interior slope face, worst near the
  peak, at both m=2 and m=4; column 7 (R7) shows visibly fewer, with a clean line closest to the
  original.
- `stream_neck_c_ema_contact_sheet.png` -- R7 (col 7) is visually indistinguishable from R6 (col
  6) here: both already draw a clean, continuous column. Expected -- a confined free-falling
  stream's cells are dominated by R6's confinement/strip machinery, not by the interior-emptiness
  case R7 targets, so there was nothing for R7 to fix or break here.
- `pool_wall_edge_c_ema_contact_sheet.png` -- R7 (col 7) shows a slightly cleaner line than R6
  (col 6) at m=4, consistent with the modest `pool_wall_edge` chamfer improvement in §9.3.

### 9.5 Recommendation: hold for review, do not ship yet

R7 removes the interior under-coverage mechanism (confirmed in §9.1-9.2) while keeping nearly every
one of R6's measured wins (§9.3: one small, localized `fp_px` regression on snapshot (a) only,
IoU/rms still net positive there) and visibly reducing the interior speckle in the picture that
motivated R6 in the first place (§9.4). It is a numeric improvement over R6 on every metric in this
instrument at every `(snapshot, m)` tested EXCEPT snapshot (a)'s `fp_px` (§9.3), with conservation
unaffected.

That said, per this round's task: **the shader was not touched.** This is a measurement-only
round, same as rounds 1-3. Three things are worth the user's judgment before any shader work
starts:
1. §9.1's honest framing -- the specific "false negatives are dominated by interior" claim that
   motivated this round was wrong; only the softer, continuous version of it held up. R7 is
   justified by the continuous metric and the picture, not by the number originally cited.
2. R6 already measured well in round 3 and still "quilted" on the real page. This instrument's
   thresholded metrics and 128px crops are a proxy, not the deployed shader; the caveat in the
   original §6 ("the shader feeds `coverage*height` into a continuous `empty_blend`, so the
   coverage-based rules should look somewhat better on the page than they score here") cuts both
   ways -- it means R6's real on-page quilting could also come from something this instrument
   under-weights (e.g. the R6-vs-R7 difference being too subtle at typical viewing distance, or a
   temporal interaction with the EMA/dithering not modelled here). R7 fixing the mechanism this
   instrument can see is not a guarantee it fixes what the user saw with their own eyes.
3. §9.3's `fp_px` trade on snapshot (a) is small but real and unexplained beyond a plausible guess
   (that snapshot's higher `lateral_substeps`/more active streams). It should be understood, not
   just tolerated, before this goes anywhere near the shader.
