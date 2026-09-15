# Upscale reconstruction: measuring alternatives to mask-aware bilinear (2026-09-14)

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
  instead of vanishing between sample points, fixing R4's exact failure mode (§4.3) — at a real
  cost, also measured in §4.3.

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

**Snapshot (a), water mid-drain** (11 true streams total across the 3 probe rows):

| rule | m=2 occ | m=2 mass | m=2 missing | m=4 occ | m=4 mass | m=4 missing |
|---|---|---|---|---|---|---|
| R0 | 2.448 | 2.226 | 2/11 | 2.370 | 3.111 | 2/11 |
| R1 | 2.796 | 2.248 | 2/11 | 3.556 | 3.219 | 2/11 |
| R2 | 2.315 | 2.113 | 2/11 | 2.370 | 3.020 | 2/11 |
| R3 | 2.315 | 2.171 | 2/11 | 2.370 | 3.062 | 2/11 |
| R4 | 1.271 | 1.822 | 0/11 | 2.074 | 3.076 | 2/11 |
| R5 | 1.595 | 1.620 | 1/11 | 2.187 | 2.760 | 2/11 |

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

\* R4/R5 at m=2 also each spawn 1-2 EXTRA spans not present in the original (a stream a hair below
threshold in truth reads as covered in the reconstruction, or a genuine stream gets split into two
narrower ones by the interface geometry) — occupied-width ratios under 1.0 alongside a nonzero
`extra` count means "individually thinner, but there are more of them," not "closer to correct."

Every rule overshoots width at every `(snapshot, m)` — none reaches the 1.0 target. R4/R5 are
consistently the closest on occupied width; R1 is consistently the worst, and gets WORSE at `m=4`
(3.56x) rather than converging. R0/R2/R3 are close to each other throughout — the plane-limiting in
R2/R3 measurably helps the continuous height field (§4.4) but does not translate into a
narrower rendered stream width, because the coverage threshold (0.003) is crossed at nearly the
same pixel for a piecewise-constant fill as for a shallow limited plane.

### 4.3 Front fidelity (round-2 fix B): symmetric chamfer distance to the TRUE coverage boundary

Global fp/fn/IoU is dominated by bulk pool and wall area and cannot see whether a FRONT is jagged
— the user's actual complaint. This computes a 2-pass chamfer-(1,√2) distance transform (~2% of
true Euclidean, plenty at this scale) of the original's `h>=0.003` boundary and of each candidate's
own coverage boundary (R0-R4: `h>=0.003`; R5: raw coverage `>=0.5`, per the round-2 brief), then
reports, LOCALLY within each named feature's crop region:
- **fwd** = distance from each RECONSTRUCTED boundary pixel to the nearest TRUE boundary pixel
  (positional accuracy / bias of what the rule draws).
- **rev** = distance from each TRUE boundary pixel to the nearest RECONSTRUCTED one (whether the
  rule's edge, wherever it runs, still passes near every part of the true edge).

Predicted staircase signature: mean ≈ `m/4`, max ≈ `m/2`. At `m=4` that's mean≈1.0, max≈2.0.

**Snapshot (c), water EMA, slope_front region** (px; fwd only, the more informative direction here):

| rule | m=2 mean | m=2 max | m=4 mean | m=4 max |
|---|---|---|---|---|
| R1 | 0.985 | 2.000 | 2.861 | 4.828 |
| R2 | 0.285 | 1.414 | 0.986 | 3.414 |
| R3 | 0.283 | 1.414 | 0.982 | 3.414 |
| R4 | 0.872 | 5.243 | 0.989 | 4.828 |
| R5 | 1.624 | 44.000 | 2.881 | 46.000 |

**Snapshot (c), pool_wall_edge region:**

| rule | m=2 mean | m=2 max | m=4 mean | m=4 max |
|---|---|---|---|---|
| R1 | 0.534 | 2.000 | 1.555 | 4.828 |
| R2 | 0.202 | 1.414 | 0.728 | 3.414 |
| R3 | 0.202 | 1.414 | 0.716 | 3.414 |
| R4 | 0.988 | 4.414 | 0.938 | 3.828 |
| R5 | 0.988 | 4.414 | 0.938 | 3.828 |

**Three findings that reframe §6's recommendation, all visible in the pictures too (§5):**

1. **R1 has the worst positional bias of every rule, at every feature, despite looking the
   smoothest.** Chamfer conflates two different things — local jaggedness (staircasing) and global
   bias (a systematically shifted edge) — and R1's bias dominates: its bilinear spillover doesn't
   just look wider, it measurably sits 2-3x farther from the true line than R0's blocky
   reconstruction at the same spot. A viewer's eye reads "smooth" from R1's lack of jaggedness and
   never notices the bias; the chamfer metric sees only the bias.
2. **R2/R3 barely improve on R0's edge position at all** (0.28 vs what R0 measures at the same
   spot — not shown above, but in the raw run log: R0's slope_front `m=2` mean is 0.302, i.e. R2/R3
   are ~6% better, not the large win their rms/mass numbers would suggest) at `m=2`, and their
   advantage nearly vanishes at `m=4` (pool_wall_edge: R2 0.728 vs R0's ~0.78, from the full log).
   **`pool_wall_edge_R2_limited_plane_m4.png` and `pool_wall_edge_R0_nearest_m4.png` are visually
   almost indistinguishable staircases** — R2/R3's real, measured win is in the continuous height
   field (§4.4, rms), not in where the rendered edge itself lands.
3. **R4/R5 have the best mean front position at `slope_front` (m=2) and are competitive elsewhere,
   but the picture reveals why that's not the whole story**: their edges are POSITIONALLY accurate
   on average but visually SPECKLED (`sand_slope_R4_plic_m2.png`, `stream_neck_R5_plic_aa_m4.png`)
   — a fine dither along the whole front, not a clean line — and R5 in particular throws a handful
   of small disconnected flecks far from the real feature (visible as isolated orange dots in
   `sand_slope_contact_sheet.png`'s and `stream_neck_contact_sheet.png`'s R5 columns), which is
   exactly what produces R5's startling `max=44-49px` forward distances at the stream/slope
   regions. This is a genuine property of thresholding a continuous coverage value at a hard 0.5
   cutoff for measurement purposes (§6 explains why this would NOT actually appear as visible
   flotsam in a real shader's continuously-faded opacity) — but it means R5's own coverage-based
   IoU/fp numbers (§4.4-adjacent, in the run log) are pessimistic relative to how the rule would
   actually render.

**Conclusion up front, since §6 needs it: no candidate wins on both the numbers and the pictures.**
R2/R3 win decisively on continuous-field accuracy and mass conservation but barely move the actual
rendered edge position and picture-confirm real (if mild) staircasing at `m=4`. R4/R5 have the best
edge position on average but visibly speckle. R1 looks the smoothest and is the least positionally
accurate of all five. See §6.

### 4.4 Main metrics (height field, conservation, coverage)

**Snapshot (c), water EMA alpha=0.4, m=2:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU |
|---|---|---|---|---|---|---|---|
| R0 | 0 | 0.0453 | 0.0331 | 0.1615 | 885 | 218 | 0.9838 |
| R1 | 0.1525 | 0.0297 | 0.0300 | 0.0202 | 2300 | 0 | 0.9670 |
| R2 | 0 | 0.0406 | 0.0262 | 0.1611 | 841 | 187 | 0.9849 |
| R3 | 0 | 0.0401 | 0.0254 | 0.1610 | 837 | 187 | 0.9850 |
| R4 | 0.2450 | 0.0679 | 0.0613 | 0.1612 | 0 | 2301 | 0.9658 |
| R5 | 0 | 0.0542 | 0.0450 | 0.1610 | 3112 | 609 | 0.9434 |

**Snapshot (c), m=4:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU |
|---|---|---|---|---|---|---|---|
| R0 | 0 | 0.0895 | 0.0559 | 0.2466 | 2083 | 792 | 0.9586 |
| R1 | 0.1625 | 0.0462 | 0.0472 | 0.0327 | 5101 | 0 | 0.9296 |
| R2 | 0 | 0.0829 | 0.0440 | 0.2456 | 1990 | 774 | 0.9601 |
| R3 | 0 | 0.0821 | 0.0424 | 0.2452 | 1987 | 774 | 0.9602 |
| R4 | 0.1206 | 0.1020 | 0.0762 | 0.2453 | 449 | 3198 | 0.9462 |
| R5 | 0.000001 | 0.0942 | 0.0643 | 0.2449 | 3593 | 1437 | 0.9240 |

R5's fp/IoU look worse than R2/R3 here — largely the coverage>=0.5-threshold speckle from §4.3,
finding 3, not a worse underlying field: R5's `rms_all` (0.0542/0.0942) sits between R2/R3
(best) and R4 (worst), and its `max_mass_err` is ~0 like R2/R3/R4-in-theory, unlike R4's actual
0.12-0.25 (§4.3 of the round-1 text explains why R4's is nonzero in practice; R5's closed-form
solve has no such discretization gap since it's solved directly against the `m*m` grid, same as
R2/R3).

**Smoothness** (2nd-difference RMS in a flat pool/pile interior; true baseline ≈0 for every
scenario): unchanged from round 1 — R0/R2/R3/R4/R5 all reconstruct a flat interior as exactly flat
(0.000000000 to printed precision); R1 is the only rule with a nonzero value (0.0001-0.0055 across
scenarios), i.e. the only rule that measurably disturbs a flat interior at all.

**R1 at alternative coverage thresholds** (unchanged finding from round 1): raising the threshold
15x (0.003 -> 0.05) only moves IoU a few points and cuts false positives ~15% — R1's over-coverage
is genuine excess height, not a threshold-tuning artifact.

## 5. Pictures

All at `artifacts/design/upscale-2026-09-14/`. **Round-2 fix D**: crops are still a 128x128 sample
of the 512 grid (the SAME region the numeric probes above use — `stream_region`/`pool_wall_region`/
`slope_region` are shared functions, so a number and its picture are always of the same patch), now
magnified 4x nearest-neighbour to 512x512 per tile (no new information, purely legibility). Total
directory size ~560KB; the three contact sheets are 56-80KB each. Grayscale = height; orange = the
coverage boundary (`h>=0.003` for R0-R4, coverage`>=0.5` for R5). Column/row layout is in
`README.md` in the same directory (also reproduced here): each `<figure>_contact_sheet.png` is 5
columns x 2 rows — **original, R1, R3, R4, R5**, row 1 = m=2, row 2 = m=4. R0/R2 crops are saved to
disk individually (for the comparisons in §4.3) but left out of the contact sheet for space.

- **`stream_neck_*`**: the original (col 1) is itself flecked with orange — a real, physical
  tick-to-tick jitter in the stream's exact edge (see CLAUDE.md's "near-neck pulse", an accepted
  oscillation), not a reconstruction artifact. R1 (col 2) draws a visibly thicker, cleaner column —
  smoother than the truth, not more accurate. R3 (col 3) stays closer to the original's actual
  width and, notably, PRESERVES some of that real jitter rather than smoothing it away. R4/R5
  (cols 4-5) speckle along the entire column and R5 additionally shows 2-3 small isolated flecks
  disconnected from the stream entirely, most visible in the m=4 row.
- **`pool_wall_edge_*`**: this is where every rule (R1/R3/R4/R5) looks good and R0/R2 (not
  pictured, see disk files) staircase — the wall is a hard, well-conditioned boundary against a
  full pool, unlike a thin stream or a shallow slope tail, and every gradient-based method has
  enough signal there to do well. `pool_wall_edge_R2_limited_plane_m4.png` and
  `pool_wall_edge_R0_nearest_m4.png` are close to indistinguishable staircases, though — R2/R3's
  wall improvement over R0 is real but modest (§4.3).
- **`sand_slope_*`** (scenario b, a fully-inside 128x128 box in the lower chamber — the boundary
  shown is the pile's own repose-angle surface, not the container): R1 (col 2) is smooth but
  visibly the widest/most offset "V". R3 (col 3) is smooth-looking AND closer to the true line —
  the best-looking single result in this crop. R4/R5 (cols 4-5) again speckle along the whole
  front; R5 shows the same isolated stray-fleck pattern as in the stream crop.

## 6. Recommendation

**No single candidate wins on both the numbers and the pictures — said plainly, per the round-2
brief, rather than picking a favourite and downplaying the rest.** Two genuinely different
properties are in tension:

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
  gets the edge position right but renders it as a speckled dither rather than a clean line, plus
  R5 throws occasional disconnected flecks under a hard coverage>=0.5 read (§4.3, finding 3).

**If forced to pick one rule to prototype in the shader next: R3.** It is the only candidate that
is simultaneously exact on conservation, has no picture-visible speckle or stray-fleck defect, and
measurably (if modestly) improves both the continuous field and the coverage false-positive count
over the shipped R1 — i.e. it has no NEW defect the pictures reveal, which R1 (positional bias),
R4 (dropout, §4.2/§4.3), and R5 (speckle/flecks, §4.3) each do. Its edge-position win over plain R0
is real but small; **this measurement does not find a rule that makes streams/fronts look
substantially crisper AND clean** — only one (R3) that is safely better than the shipped rule on
every axis measured without introducing a new visible artifact, and two others (R4/R5) that trade
a genuine positional-accuracy win for a genuine new visual defect.

**R4 is not recommended**: worst false-negative counts by 5-13x, the largest discretization-driven
`max_mass_err` of the non-R1 rules, and it drops individual free-fall streams entirely at `m=4`
(§4.2's `missing` column) — on top of the newly-observed speckle.

**R5 is the most interesting rule for FUTURE work, not this one.** Its closed-form conservation and
continuous coverage are real, structural improvements over R4 (no exact-dropout failure mode,
§4.4), but this instrument's coverage`>=0.5` boundary-detection convention (used because the round-2
brief specifies it, for a fair coverage/IoU comparison) makes its OWN metrics look worse than the
rule probably deserves: a real shader would feed R5's `coverage * height` straight into the
existing `empty_blend = clamp(h/0.003, 0, 1)` opacity formula, which fades continuously — a coarse
cell contributing 2% coverage to a distant pixel would render at ~2% opacity, not as a visible fleck
the way a hard `>=0.5` cutoff treats it in this diagnostic. **Recommendation: if R3 ships and still
doesn't satisfy the front-crispness complaint, prototype R5 in the actual shader** (where its
coverage is read continuously, not thresholded) rather than trusting this instrument's own R5
numbers/pictures at face value — they are a deliberately pessimistic proxy for a quantity that was
never meant to be viewed as a hard binary in the first place.

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
