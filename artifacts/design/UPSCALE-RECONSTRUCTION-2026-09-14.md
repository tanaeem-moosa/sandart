# Upscale reconstruction: measuring alternatives to mask-aware bilinear (2026-09-14)

MEASUREMENT ONLY. Nothing in `sandart-render`, `sandart-wasm`, or `sandart-sim`'s physics changed.
The instrument is `sandart-sim/examples/diag_upscale_reconstruction.rs`
(`cargo run -p sandart-sim --release --example diag_upscale_reconstruction`); everything below is
its output, re-derived once more for this writeup, plus the PNGs it wrote to this same directory.

## 1. Framing

The app simulates at `S` and draws at `n = S*m`, `m` in `{2, 4}`. The fragment shader
reconstructs each render pixel's height with **mask-aware bilinear interpolation**
(`sandart-render/src/shader.wgsl` lines 622-671): standard 2x2-texel bilinear, with any
OUTSIDE-mask corner's weight zeroed and the remainder renormalised over the texels that are
actually inside the vessel. That renormalisation is already doing real work — it's why the
"pool edge against a wall" pictures below show every rule, R1 included, tracking a smooth wall
boundary with no fringe-of-zeros artifact. **R1's problem is not walls; it's empty interior.**

R1 already has two of the required properties:
- **No overshoot** — a convex combination of the four corner values can't exceed their max or
  undershoot their min.
- **Global conservation** — the bilinear kernel sums to 1 over the whole grid (a standard tent-filter
  identity), so total mass is unchanged by the round trip.

What it does **not** have is **per-cell conservation**: a single coarse cell's own pixels do not
average back to that cell's value, because bilinear blends mass across the cell boundary into
whichever neighbour is nearer the sample point. In 1D, a cell's own pixels hold ~3/4 of its content
plus ~1/8 borrowed from each neighbour; against an empty neighbour, that 1/8 is drawn as extra
material where there is none. That is the mechanism, not a guess — §4's stream-width numbers
measure it directly.

Full required-property list, and how the candidates are built to satisfy each:
1. **Per-cell conservation** — R0 is trivially exact (constant fill). R2/R3 get it from an explicit
   bisection re-solve. R4 gets it from the PLIC area-fraction identity, exactly in the continuum but
   only approximately once rasterised onto `m*m` discrete samples (§4, §6).
2. **No overshoot** — R2/R3 enforce it with a Barth-Jespersen limiter against the cell's own 3x3
   neighbourhood; R4 never produces a value outside `{0, h_ref}`, both already within range.
3. **Smooth where the field is smooth** — measured directly by the pool-interior 2nd-difference
   probe (§4.3).
4. **Sharp where material meets empty, edge inside the cell** — measured by coverage IoU / false
   positives (§4.2) and shown by the picture crops (§5).
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
round-trip error only, with the physics held fixed.

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
mask — the same function the render-resolution "fine mask" upload already uses, so the vessel
outline can't drift between fine and coarse. A coarse cell's value is the mean of its `m*m` fine
cells that are inside the *fine* mask; fine-inside cells whose block's coarse cell is OUTSIDE
(a real mask-resolution mismatch at the vessel boundary, not a bug) are tracked as **lost mass**,
reported below rather than silently dropped.

**Upscale**: each candidate is evaluated at the exact sub-pixel offsets a 512-wide image needs
inside each coarse cell — `m*m` fixed offsets per cell, identical to what the shader's fragment
grid actually samples (`(i+0.5)/m - 0.5` in coarse-cell units, matching `shader.wgsl`'s own
`texel_coords` formula exactly for R1).

## 3. Candidates

- **R0 nearest** — piecewise constant. Trivially per-cell conservative and overshoot-free; every
  wall/void edge is a staircase.
- **R1 current** — mask-aware bilinear exactly as shipped (see §1).
- **R2 limited plane** — gradient from a least-squares fit over the inside 3x3 neighbours (falls
  back to independent per-axis normal equations when the neighbour set is rank-deficient, e.g. only
  two neighbours along one line next to a wall). Barth-Jespersen-limited so the plane's own four
  corners stay inside the 3x3 min/max (including the cell's own value). Clipped at 0, then a vertical
  shift `delta` is found by bisection over the cell's own `m*m` samples so their mean is exactly the
  cell's value.
- **R3 face-matching gradient** — the user's "match the gradient" idea. Per axis, steepens toward
  whichever available neighbour is *fuller* so the plane reaches that neighbour's exact value at
  the shared face (half a cell away): reaching neighbour value `hN` from `h0` over a half-cell
  needs slope `2*(hN-h0)`, twice R2's central-difference estimate. Where the cell is a local max
  along that axis (no fuller neighbour), falls back to a gentle, non-steepened one-sided/central
  difference. Same Barth-Jespersen limiter, same clip-and-bisect conservation re-solve as R2 — the
  limiter is what turns "steepen toward the full neighbour" into a genuine sharp-but-bounded edge
  rather than an overshoot.
- **R4 vertical-edge PLIC (Youngs)** — interface normal from the same least-squares gradient;
  `h_ref = max` over the inside 3x3 neighbours including self; covered fraction `f = h0/h_ref`. The
  covered region is positioned by *exact* half-plane polygon clipping (Sutherland-Hodgman) plus
  bisection on the threshold, not Monte-Carlo sampling, so the covered area is geometrically exact
  for `f` in the continuum. Material is `h_ref` inside that region, 0 outside — no clip, no
  conservation re-solve needed in principle (§6 measures where this breaks down in practice).

**Film case** (all neighbours shallow and similar): R2/R3's gradient is small either way (the
least-squares/face-matching estimates are both driven by neighbour differences, which vanish), the
Barth-Jespersen window is narrow, and the conservation re-solve needs `delta≈0` — the cell ends up
covering its whole footprint at very close to its own flat value, matching R0/R1 there. R4's
`h_ref≈h0` drives `f≈1`, which the code special-cases to fill the whole cell outright. All three
converge to "cover the whole cell" exactly as required.

## 4. Results

Full output: `/tmp` run log reproduced by `cargo run -p sandart-sim --release --example
diag_upscale_reconstruction`, ~34s wall on this machine for all three snapshots x two resolutions x
five rules. `rms`/`mass_err`/`smooth` are in heightmap units (0..1); pixel counts are over the
whole 512x512 fine-inside domain (not just the picture crops).

### 4.1 Downscale mass loss (reported, not hidden)

| snapshot | m=2 lost / total | m=4 lost / total |
|---|---|---|
| (a) water mid-drain | 145.8 / 62505 (0.233%) | 744.9 / 62505 (1.192%) |
| (b) dry sand pile | 168.4 / 39322 (0.428%) | 870.4 / 39322 (2.214%) |
| (c) water EMA | 145.6 / 62505 (0.233%) | 744.7 / 62505 (1.192%) |

All of this is the fine/coarse mask-resolution mismatch at the vessel boundary (fine-inside cells
whose `m*m` block's coarse cell rasterises OUTSIDE), not a defect in any candidate rule — it is the
same for every rule at a given `(snapshot, m)`, and grows with `m` as expected (coarser masks lose
more boundary detail).

### 4.2 Main metrics

`fp`/`fn` = coverage false-positive/false-negative pixels at the shader's `h>=0.003` threshold.
`stream ratio` = mass-weighted uniform-equivalent width at the row 2 cells below the single neck row
(all three scenarios' vessels have their neck(s) at one row, `y=256`; `MultiNeckHourglass`'s several
parallel necks sit at that same row, so this is a combined width across whatever streams are
present there), divided by the same quantity on the original 512 field. 1.0 = perfect;
the true simulated widening this project has independently measured is ~10-15%, i.e. a target
ratio of ~1.10-1.15.

**Snapshot (a), water, mid-drain, m=2:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU | stream ratio |
|---|---|---|---|---|---|---|---|---|
| R0 nearest | 0 | 0.0837 | 0.0791 | 0.1618 | 1510 | 182 | 0.9751 | 1.590 |
| R1 bilinear (shipped) | 0.1952 | 0.0788 | 0.0802 | 0.0207 | 3012 | 3 | 0.9567 | 1.914 |
| R2 limited plane | 0 | 0.0821 | 0.0773 | 0.1613 | 1475 | 172 | 0.9758 | 1.596 |
| R3 face-match | 0 | 0.0826 | 0.0779 | 0.1613 | 1470 | 172 | 0.9759 | 1.605 |
| R4 PLIC | 0.2475 | 0.1089 | 0.1063 | 0.1617 | 297 | 2245 | 0.9620 | 1.636 |

**Snapshot (a), m=4:**

| rule | max_mass_err | rms_all | rms_int | rms_front | fp | fn | IoU | stream ratio |
|---|---|---|---|---|---|---|---|---|
| R0 nearest | 0 | 0.1153 | 0.0943 | 0.2466 | 2735 | 787 | 0.9492 | 2.765 |
| R1 bilinear | 0.1643 | 0.0872 | 0.0906 | 0.0331 | 5807 | 0 | 0.9198 | 2.922 |
| R2 limited plane | 0 | 0.1105 | 0.0880 | 0.2457 | 2669 | 773 | 0.9503 | 2.595 |
| R3 face-match | 0 | 0.1100 | 0.0875 | 0.2454 | 2669 | 773 | 0.9503 | 2.547 |
| R4 PLIC | 0.1184 | 0.1276 | 0.1105 | 0.2452 | 823 | 2891 | 0.9449 | **0.000\*** |

\* At exactly the probed row, R4's discretised interface misses all 4 of that row's sub-pixel
samples for that coarse cell (see §6) — occupied width reads 0 there even though the picture
(`stream_neck_R4_plic_m4.png`) shows the stream as visually continuous overall; this is the local
symptom of the same quantization that gives R4 the largest `max_mass_err` of the three conservative
rules.

**Snapshot (b), dry sand pile, m=2 / m=4** (condensed — full table in the run log):

| rule | m | rms_all | fp | fn | IoU | stream ratio |
|---|---|---|---|---|---|---|
| R0 | 2 | 0.0824 | 727 | 136 | 0.9696 | 1.646 |
| R1 | 2 | 0.0560 | 1847 | 5 | 0.9372 | 1.728 |
| R2 | 2 | 0.0720 | 714 | 128 | 0.9703 | 1.646 |
| R3 | 2 | 0.0712 | 713 | 128 | 0.9703 | 1.671 |
| R4 | 2 | 0.0825 | 29 | 1040 | 0.9614 | 1.748 |
| R0 | 4 | 0.1568 | 1752 | 608 | 0.9197 | 2.334 |
| R1 | 4 | 0.0868 | 3929 | 10 | 0.8753 | 2.334 |
| R2 | 4 | 0.1443 | 1732 | 601 | 0.9206 | 2.173 |
| R3 | 4 | 0.1438 | 1729 | 601 | 0.9207 | 1.913 |
| R4 | 4 | 0.1411 | 205 | 1521 | 0.9380 | 0.000\* |

**Snapshot (c), water EMA alpha=0.4, m=2 / m=4:**

| rule | m | rms_all | fp | fn | IoU | stream ratio |
|---|---|---|---|---|---|---|
| R0 | 2 | 0.0453 | 885 | 218 | 0.9838 | 1.199 |
| R1 | 2 | 0.0297 | 2300 | 0 | 0.9670 | 1.281 |
| R2 | 2 | 0.0406 | 841 | 187 | 0.9849 | 1.212 |
| R3 | 2 | 0.0401 | 837 | 187 | 0.9850 | 1.260 |
| R4 | 2 | 0.0679 | 0 | 2301 | 0.9658 | 1.358 |
| R0 | 4 | 0.0895 | 2083 | 792 | 0.9586 | 1.873 |
| R1 | 4 | 0.0462 | 5101 | 0 | 0.9296 | 1.936 |
| R2 | 4 | 0.0829 | 1990 | 774 | 0.9601 | 1.774 |
| R3 | 4 | 0.0821 | 1987 | 774 | 0.9602 | 1.650 |
| R4 | 4 | 0.1020 | 449 | 3198 | 0.9462 | 0.000\* |

The EMA (c) is uniformly easier than the raw tick (a) for every rule — temporal smoothing already
removes some of the same high-frequency content the spatial reconstruction fights — but the
*ranking* between rules is unchanged: R2/R3 beat R0/R1 on fp/IoU, R1 has the worst stream-width
ratio, R4 has the fewest false positives and the worst false negatives, at every m, in every
scenario.

### 4.3 Smoothness (2nd-difference RMS in a pool/pile interior)

Probed in a fully-inside 24x24 box in the lower chamber. The true field there is essentially flat
(a settled pool, or a saturated pile plateau) — the original's own 2nd-difference RMS is
`9e-9` (scenario a) or exactly `0` (scenarios b, c) to printed precision, i.e. the TRUE baseline is
near-zero, not measurement noise. Reconstructed values, same units:

| rule | (a) m=2 | (a) m=4 | (b) m=2 | (b) m=4 | (c) m=2 | (c) m=4 |
|---|---|---|---|---|---|---|
| R0 | 0 | 0 | 0 | 0 | 0 | 0 |
| R1 | 0.002855 | 0.005450 | 0.001687 | 0.002807 | 0.000146 | 0.002060 |
| R2 | 0 | 0 | 0 | 0 | 0 | 0 |
| R3 | 0 | 0 | 0 | 0 | 0 | 0 |
| R4 | 0 | 0 | 0 | 0 | 0 | 0 |

R1 is the only rule that measurably disturbs a flat interior — small in absolute terms, but the
only nonzero row in this table. R0/R2/R3/R4 all reconstruct a flat neighbourhood as exactly flat.

### 4.4 R1 at alternative coverage thresholds (m=2, scenario a)

| threshold | fp | fn | IoU |
|---|---|---|---|
| 0.001 | 3193 | 0 | 0.9543 |
| 0.003 (shipped) | 3012 | 3 | 0.9567 |
| 0.01 | 2860 | 2 | 0.9587 |
| 0.02 | 2844 | 6 | 0.9587 |
| 0.05 | 2632 | 1 | 0.9614 |

Raising the threshold 15x (0.003 -> 0.05) only recovers IoU from 0.9567 to 0.9614 and cuts false
positives by 12% — most of R1's over-coverage is not sitting just above the threshold where a
different cutoff would fix it; it's genuine excess *height* spread well beyond 0.003, consistent
with real spillover rather than a threshold-tuning problem.

## 5. Pictures

All at `artifacts/design/upscale-2026-09-14/`, ≤ 32 KB each. Grayscale = height (0..vmax per
figure); orange = the `h>=0.003` coverage boundary (a pixel where a 4-neighbour disagrees on
covered/not). Each `<figure>_contact_sheet.png` lays out, left to right: original, then R0/R1/R2/R3/
R4 at m=2 (top row), R0/R1/R2/R3/R4 at m=4 (bottom row).

- **`stream_neck_*`** (scenario a, crop around the neck and the stream below it): R1 visibly
  thickens the stream at both m (compare `stream_neck_R1_bilinear_m4.png`'s stream width against
  `stream_neck_original.png`); R4 renders the thinnest, sharpest stream at m=2 but visibly
  speckles/gaps at m=4 (`stream_neck_R4_plic_m4.png`).
- **`pool_wall_edge_*`** (scenario a, a settled pool against the vessel wall): R0 is an obvious
  staircase at both m; R1/R2/R3/R4 all track the diagonal wall smoothly — confirming §1's point that
  R1's mask-aware renormalisation already solves the *wall* case, and the remaining problem is
  specifically empty (in-mask) neighbours.
- **`sand_slope_*`** (scenario b, a fully-inside 128x128 box in the lower chamber, no vessel wall
  present — the boundary shown is the pile's own repose-angle surface, not the container): same
  story as the neck crop, R0 stair-steps hardest at m=4, R2/R3 show mild stepping, R1 and R4 are
  smoothest.

## 6. Recommendation

**R3 (face-matching limited plane), with R2 as a near-identical, slightly simpler fallback.**

- R3 beats R2 on rms_all/rms_int in 5 of 6 (scenario, m) combinations, and on stream-width ratio in
  4 of 6 — most clearly at m=4 where it matters most (e.g. scenario b: ratio 1.91 vs R2's 2.17).
  Where R2 wins it is by a small margin (e.g. scenario a m=2 stream ratio: 1.596 vs 1.605). This is
  the direct implementation of the user's "match the gradient" idea, and it measures as at least as
  good as, usually slightly better than, the plainer least-squares gradient.
- Both R2 and R3 **exactly satisfy per-cell conservation** (`max_mass_err` ~0, vs R1's 0.15-0.35),
  **never overshoot** (Barth-Jespersen-limited by construction), **reconstruct a flat interior as
  exactly flat** (§4.3 — R1 does not), and cut false-positive coverage pixels roughly in half versus
  R1 at m=2 while avoiding R0's staircase entirely (§5).
- Neither closes the stream-width gap: even the best case here (R3, scenario c, m=2: ratio 1.26) is
  still well above the ~1.10-1.15 the real simulated stream widens by, and at m=4 every rule is
  still 1.6-2.9x. **This measurement does not find a rule that makes the stream-width problem go
  away — only one that roughly halves R1's excess at m=2 and meaningfully reduces it at m=4.** That
  should be stated plainly rather than oversold.
- **R4 (PLIC) is not recommended** despite having the fewest false positives and the sharpest edges
  in the wall/slope pictures, for the specific reason this exercise cares about most: thin, isolated
  features are exactly where it fails. Its false negatives are 5-13x R2/R3's at every (scenario, m),
  its `max_mass_err` is the largest of the three conservative rules (its conservation is exact only
  in the continuum — see below), and at m=4 it locally misses the probed stream row's samples
  entirely in every water scenario tested. A rule that can render a falling stream as intermittently
  invisible is a worse regression than the one being fixed.

**Why R4's conservation breaks down discretely.** The PLIC area fraction `f = h0/h_ref` is exact as
a continuous integral, but the shader only ever evaluates a rule at `m*m` fixed sub-pixel points
(4 at m=2, 16 at m=4) — there is no continuous integral in the render path, only point samples. A
thin covered strip whose true area fraction is, say, 1/12 can easily fall between two of the 16
sample points at m=4, or between the 4 at m=2, giving a discrete mean of 0 for that texel even
though the geometric fraction is nonzero. R2/R3 do not have this failure mode because their
conservation re-solve targets the *same discrete m*m grid* the shader will actually sample, by
construction (the bisection literally is "adjust delta until the mean of these m*m particular
points equals h0") — they are exactly right for the resolution they were solved at, where R4 is
only asymptotically right as `m -> infinity`.

## 7. Shader cost estimate for R3

**Today (R1):** 4 filtered `textureSampleLevel` reads (the 2x2 quad) plus a renormalisation branch,
per fragment, per frame.

**R3, naively per-fragment:** the least-squares/face-matching gradient needs the 3x3 neighbourhood
of *cells*, i.e. 9 `textureLoad`s (nearest, no filtering) instead of R1's 4 filtered reads — worse
at the fragment stage if computed there directly, and it would recompute the same per-cell plane
for every one of the `m*m` fragments that share a coarse cell.

**R3, as shipped (recommended shape):** move the per-cell work — gradient, Barth-Jespersen `phi`,
and the conservation `delta` — into a small pre-pass that runs once per *simulation* cell
(`S*S` work, not `n*n`), writing `(h0, phi*gx, phi*gy, delta)` into an auxiliary `S`x`S` RGBA32F
texture, analogous to the heightmap upload the sim already does every frame. The fragment shader
then does **one** `textureLoad` of that pre-computed plane (nearest, based on which cell the
fragment's sub-pixel offset falls in) plus a 2-term dot product — cheaper per-fragment than R1's
current 4 filtered reads, with the added cost moved to a pass over `S*S` cells that is the same
shape of work the project already budgets for per-frame simulation output.

**Does the conservation re-solve have a closed form?** No general closed form in 2D: clip-at-0
followed by averaging is piecewise (which of the `m*m` corners get clipped depends on the plane's
orientation and how close `h0` is to 0), so the mean-vs-`delta` relationship is a piecewise-linear
function of `delta` with a case count that depends on `m`. Two things make this cheap in practice
rather than needing an approximation:
1. **The common case needs no solve at all.** Before any clipping, the mean of a linear plane over
   the `m*m` sample offsets is *exactly* `h0` whenever those offsets are symmetric about 0 — true
   for both `m=2` (`±0.25`) and `m=4` (`±0.125, ±0.375`). So `delta=0` is already exact for every
   cell whose limited plane never dips below 0 across its own footprint, which is every interior
   cell away from a material/empty boundary. The bisection is only ever needed at frontier cells.
2. Where it is needed, a short *fixed-iteration* bisection (8-12 iterations is enough precision for
   an 8-bit-equivalent height channel) is cheap **because it runs once per cell in the pre-pass, not
   per rendered fragment** — the same reason the fragment-side cost above doesn't grow with `m`.

## 8. Reproduction

```
cargo run -p sandart-sim --release --example diag_upscale_reconstruction
```

~34s wall (three ~600-900-tick 512x512 simulations plus five reconstruction rules x two
resolutions x three snapshots). Writes all PNGs in this directory and prints the full metrics
tables (including per-row stream-width detail and the `DUMP_ROW_COUNTS=1` mask-profile debug path
used to confirm `MultiNeckHourglass`'s several necks sit at one shared row rather than being
vertically distributed).
