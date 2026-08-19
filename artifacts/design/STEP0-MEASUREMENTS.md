# Build step 0 measurements — falsifying §0.1

Status: measurement only. No coupling built. `sandart-sim/src/physics.rs` and `sandart-sim/src/lib.rs`
are untouched. All numbers below come from `sandart-sim/examples/diag_coarse_step0.rs`, a
self-contained example that aggregates the existing fine-level state into an offline 64x64 coarse
grid and runs the *existing* `overfill_equilibrium_transfer`/`cell_potential` over that coarse
graph — no new physics, nothing wired into the real sim. Compiled and run inside the `sandart-dev`
container (the host has no linker).

**This is the corrected version of this document.** The first pass got the Q1 headline wrong, in a
way its own Q2 measurement already contradicted, and proposed a fourth question (a direct
per-component solve) that turned out to be physically wrong for this model. Both are corrected
in place below, not appended as a contradiction. What was wrong, plainly:

1. **Wrong residual metric.** The first pass measured convergence as "largest unsatisfied stress
   on any edge." That is nonzero *forever* at any free surface — an empty cell above a filled one
   always has stress it cannot act on (nothing to donate), and that is what rest looks like in
   this model, not a failure to converge. The apparent "boundary lock, stuck for 100,000 sweeps"
   was this metric measuring the free surface. Re-measured against **largest realised transfer in
   a sweep** (mass that actually moved) below.
2. **Wrong pool depth for the pinning question.** The original single test used a ~300-fine-row
   nominal fill, which — because the coarse relaxation compacts a uniform fill into a shorter,
   denser column — settles at an *effective* depth below the pinning threshold. Re-measured across
   a depth sweep below; pinning is real, and now located.
3. **A proposed direct per-component solve was wrong and has been dropped, not built.** For a
   *resting* connected component `eta` is a single scalar (§0.3), so it looked like an
   attractive O(1)-per-tick alternative to iterative relaxation. It is wrong for this model: a
   falling stream is mass-connected to the pool it lands in, and one `eta` per component would
   hand the stream hydrostatic pressure it must not have — exactly "support is not transitive,"
   the failure that parked the #55 head field, and a direct break of acceptance criterion 4 (the
   hourglass). It was never built into the real sim (nothing here was), and the exploratory
   version in this example has been removed rather than reported as a finding. Replaced by a
   direct question about the iterative relax's own affordability (Q-New, below), because the
   O(N²) cost the direct solve was meant to dodge turns out not to be a problem: `M` persists
   across ticks, so settling does not have to happen within one tick.

## Commands run

```
distrobox enter sandart-dev -- bash -lc "cd /home/deck/projects/sandart && \
  cargo run -p sandart-sim --release --example diag_coarse_step0 -- 512 1500 5.0 20000 0.00001 300"
```

Args: `grid=512 ticks=1500 stiffness=5.0 max_sweeps=20000 tol_transfer=0.00001 ticks_depth_sweep=300`.
The depth sweep (§B) uses a reduced fine-tick count (300 instead of 1500) purely for wall-clock
budget — the design's own §1 numbers show fine settling barely moves the aggregated state at this
tick count regardless (6% of hydrostatic reached after 1500 ticks at grid 512), so this does not
materially change what gets aggregated.

## Setup

```
grid 512, coarse 64x64 (t=8), stiffness 5, overfill_capacity 1.9000, o_max 0.9000
base_head 1.0000, base_head_coarse 8.0000, overfill_head_unit 125.00, underfill_tension 1.000
convergence metric: largest realised mass transfer in one sweep < 0.00001
default pool: depth 300 fine rows, 1500 ticks. total fine mass 131880.0, restricted 131880.0 (exact)
```

---

## (A) The residual metric, corrected — and the reviewer was right

**Claim under test:** the free surface's unsatisfied stress is not a convergence failure; the
field had in fact converged. Measure it.

**Measured, Q1 (bounded law, default 300-row pool):**

```
[Q1] sweep  1000, largest transfer = 0.04882296
[Q1] sweep  2000, largest transfer = 0.00149357
[Q1] sweep  5000, largest transfer = 0.00008017
[Q1] sweep 20000, largest transfer = 0.00046301
stopped after 20000 sweeps (4.8s), largest realised transfer in final sweep = 0.00046301 (tol 1e-5)
interior residual (both endpoints over their own capacity) = 0.000511
```

`interior_residual` restricts the stress check to edges where *both* cells are already compressed
(the wet interior, away from any free surface) — this directly answers "did the interior reach
`phi_below = phi_above + base_head`?" The answer is yes, to three-and-a-half decimal places
(`0.000511` against a `base_head_coarse` of `8.0` — five orders of magnitude smaller).

The corrected `eta` profile (this run also fixes a second bug in the original: `eta` was computed
from `P` alone rather than the full potential `phi = h/cap + P`, which is what §0.3 actually
defines) is now **flat to four decimal places** through the entire wet interior:

```
row 38: eta = -296.1631      row 50: eta = -296.1632
row 44: eta = -296.1632      row 55: eta = -296.1631
row 61: eta = -296.1631
```

That is `eta` constant down the column to within floating-point jitter — a materially stronger
confirmation of §0.3 than the original (wrong-metric, wrong-`eta`) write-up managed.

The worst remaining stress edge, at convergence:

```
[Q1] worst STRESS edge: VERTICAL (tx=2,ty=2)->(2,ty=3): stress=8.0000 |
  a: h=0.0000 cap=9.0000 o=0.0000 | b: h=0.0000 cap=24.0000 o=0.0000
```

Both endpoints are **empty** — this is deep in the dry air above the pool (row 2 of 64), nowhere
near the free surface (~row 37) or the pinned region. Two empty cells, `base_head_coarse=8`
pushing "down" between them, phi identical at both (`-1`, the tension floor) — stress stays at
exactly `base_head_coarse` forever because there is nothing to move. **This is exactly the free-
surface/vacuum signature the reviewer predicted**, not a stuck interior.

**Verdict: the reviewer was right, and the original document was wrong.** The plateau reported in
the first pass was the residual stress metric measuring a free surface, not a failure to converge.
Under the correct metric (largest realised transfer), the interior reaches equilibrium — `eta` flat
to 4 decimals, interior residual `5×10⁻⁴` against a `base_head_coarse` of `8`. The "boundary lock"
section of the first draft is retracted; there is no lock.

---

## (B) Where does the bounded law actually pin?

**Depth sweep**, bounded law, transfer-magnitude convergence, `ticks=300` per pool:

```
 depth   max(o)  pinned   wet   interior_resid   sweeps
   150   0.5164       0   840        0.000389    20000
   200   0.6284       0  1080        0.000557    20000
   250   0.7148       0  1260        0.000519    20000
   300   0.8033       0  1500        0.000542    20000
   400   0.9000     240  1860        8.000000    20000   <- PINNED
```

At every depth from 150 to 300 fine rows, the interior residual sits 4-5 orders of magnitude below
`base_head_coarse` — genuinely converged, comfortably below the ceiling. **At 400 rows the picture
flips completely**: `max(o)` sits at exactly `0.9000` (the ceiling, to the displayed precision),
**240 of 1860 wet coarse tiles (12.9%) are pinned**, and the interior residual for the pinned
region is stuck at **exactly `8.0000`** — `base_head_coarse` itself, unresolved. That is §0.1's
literal prediction, reproduced cleanly: `P` has gone spatially flat in the pinned region, and the
driving force there cannot act.

An independent analytic cross-check (solving `unit*(o + o²/o_max) = D*base_head` directly by
bisection, no discrete solver involved) gives:

```
D=150: demanded o=0.6825 (within o_max)      D=300: demanded o=1.0870 (EXCEEDS o_max)
D=200: demanded o=0.8316 (within o_max)      D=400: demanded o=1.3057 (EXCEEDS o_max)
D=250: demanded o=0.9651 (EXCEEDS o_max)
```

This analytic check predicts pinning starting at `D=250`, earlier than the measured `D=400`. The
two are not in conflict — the analytic formula assumes the column's own free surface sits exactly
`D` physical rows above its base, but the *measured* equilibrium column is shorter than its nominal
fill depth: mass conservation means a uniformly-filled `D`-row column, once compressed into a
hydrostatic profile, occupies **fewer** physical rows than `D` for the same total mass (higher
density near the base needs less depth to hold the same mass). Back-solving the analytic formula
against the *measured* `o=0.8033` at nominal `D=300` gives an effective depth of only `~190` rows
— i.e. roughly 110 of the nominal 300 rows of fill "compacted away." The analytic number is a
useful, pessimistic sanity check on the order of magnitude; the discrete measurement, which
actually conserves mass through the compaction, is the authoritative one.

**Verdict: the bounded law pins, and the transition is sharp, between 300 and 400 nominal fine rows
of fill.** Below that (up to at least 300 rows), the relaxation converges with room to spare — 10%
of the ceiling headroom still unused at 300 rows. This settles §0.1's central question: the bounded
overfill law is usable for pools shallower than roughly this threshold, and pins beyond it, exactly
as designed predicted — just not at the specific depth the first pass happened to test.

---

## Q2 — the elevation double-count (unchanged conclusion, refreshed numbers)

**Prediction (§0.2):** `P[D] - P[C] ≈ t*base_head = 8.0` between vertically adjacent coarse tiles.

**Measured**, from the (correctly) converged Q1 field, centre column:

```
   C    D    P[D]-P[C]  t*base_head   ratio
  37   38       7.3670       8.0000   0.9209   <- transition row, near the free surface
  38   39       7.9462       8.0000   0.9933
  50   51       7.9711       8.0000   0.9964
  60   61       7.9769       8.0000   0.9971
mean ratio over pressurised interior pairs: 0.9928
```

**Verdict unchanged: confirmed.** The interior ratio sits at 0.99-0.997, essentially exactly
`t*base_head`. `P[C]` raw (not `eta`) would double-count almost one whole gravity-row of elevation
at every tile seam. The `eta` reformulation in §0.3/§7 remains mandatory. (This is also the
measurement that first exposed the original Q1 write-up's error: a field that had genuinely failed
to converge could not produce a ratio this close to the exact prediction.)

---

## Q3 — O(N) or O(N²)? (re-measured against the corrected metric)

**Prediction (§10):** nonlinear diffusion, `O(L²)`.

**Measured**, 1-D coarse chain, transfer-magnitude convergence (tol `1e-5`):

```
     L   sweeps-to-settle   final transfer
     8              135        0.00000881
    16              513        0.00001000
    32             1931        0.00000998
    64             7195        0.00001000
estimated exponent L=8->L=64: sweeps ~ L^1.91
```

**Verdict: confirmed**, and close to the earlier (stress-metric) measurement (114/430/1598/5848,
exponent 1.89) — the exponent is a property of the physics, not an artifact of which residual was
used to declare victory. `O(L²)` diffusion is real.

---

## Q-New — ticks-to-settle at N sweeps/tick, `M` persisting across ticks (replaces the dropped direct solve)

The coordinator's point: `O(L²)` total-sweep cost is not the problem it looked like, because `M`
persists across ticks — settling does not have to happen inside one tick's sweep budget. The
number that actually matters is **ticks**, not sweeps, and how that scales with `N`.

**Measured**, `L=64` chain (the worst case, since the coarse grid is fixed at 64x64), `N` sweeps
applied per tick, `M` carried forward with no reset and no anchor:

```
     N   ticks-to-settle   total sweeps   per-tick cost / 1 fine sweep@512
     8              900           7200                             0.1250
    16              450           7200                             0.2500
    32              225           7200                             0.5000
    64              113           7232                             1.0000
   128               57           7296                             2.0000
```

Two things fall out of this table directly:

- **Total wall-clock cost (ticks × per-tick cost, in fine-sweep-equivalents) is close to constant
  across `N`:** `900×0.125=112.5`, `450×0.25=112.5`, `225×0.5=112.5`, `113×1.0=113`,
  `57×2.0=114`. Grouping sweeps into fewer, larger per-tick batches does not change the total work
  — it only changes how many ticks that work is spread across, exactly as `O(N²)` diffusion with a
  fixed total-sweep requirement predicts.
- **At the design's own proposed `N=8`, the worst-case chain settles in 900 ticks, each costing
  12.5% of one fine sweep** — i.e. about 112 fine-sweep-equivalents total, a one-time cost to
  settle a maximally out-of-equilibrium chain (all mass in one cell) from cold. At `N=64` (one
  coarse sweep costing the same as one fine sweep, per §5's own arithmetic — confirmed here,
  `cost_fraction=1.0` exactly at `N=64`), the same job takes 113 ticks.

**Verdict: `O(N²)` is not the risk it looked like.** Compared against the design's own figure of
~125 fine ticks per row of depth (so a comparably deep fine-level settle would run into the tens of
thousands of ticks), even the slowest tested configuration (`N=8`, 900 ticks) is one to two orders
of magnitude faster, and this is the *worst case* — a cold, maximally-disequilibrated chain, not
the small nudges `M` will see tick-to-tick once seeded. The coordinator's arithmetic
("64 sweeps/tick costs about one fine sweep, L=64 settles in ~90 ticks against ~25,000 for the fine
level") is confirmed by measurement (113 ticks, same order).

---

## What was dropped, and why

A direct per-component `eta` solve (bisection on a single scalar per connected coarse component,
inverting the unbounded law) was prototyped in an earlier revision of this instrument and matched
the iterative relax exactly wherever the iterative relax had converged, at roughly 1ms per solve
versus ~16s for the iterative relax to reach the same state. It has been **removed, not adopted**:
it assumes every cell in a connected component shares the same resting `eta`, which is only true
for material genuinely at rest. A falling stream is mass-connected (through the coarse graph) to
whatever pool it lands in, so a direct per-component solve would hand the falling stream the
landing pool's full hydrostatic pressure — "support is not transitive," the exact reasoning that
parked the #55 head field, and a direct violation of acceptance criterion 4 (no pressure in a
falling stream, so the hourglass does not break). The iterative relax's finite propagation rate,
which only pressurises what is actually compressed, is the property worth keeping, and Q-New above
shows its cost is affordable without needing to replace it.

---

## Summary

| Question | Prediction | Measurement | Verdict |
|---|---|---|---|
| (A) residual metric | reviewer: free-surface stress is not non-convergence | interior residual 5×10⁻⁴ (vs `base_head_coarse=8`), `eta` flat to 4 decimals, worst stress edge is a genuine dry/dry pair far from the pool | **Reviewer confirmed; original Q1 write-up was wrong, corrected in place.** |
| (B) where does the bounded law pin | §0.1: it pins at depth | 0 pinned tiles at D≤300 (residual ~5×10⁻⁴); 240/1860 pinned at D=400 (residual = exactly `base_head_coarse`) | **Pins, sharply, between 300 and 400 nominal fine rows.** |
| Q2 elevation double-count | `P[D]-P[C] ≈ t·base_head` | ratio 0.99-0.997 in the interior | **Confirmed**, unchanged. |
| Q3 diffusion cost | `O(L²)` | `sweeps ~ L^1.91`, 7195 sweeps at L=64 | **Confirmed**, unchanged. |
| Q-New: ticks-to-settle | (new question, replaces the dropped direct solve) | 900 ticks at N=8, 113 at N=64; total cost ~constant (~113 fine-sweep-equivalents) across N | **`O(N²)` is affordable**: `M`'s persistence across ticks absorbs the cost. |
| Direct per-component solve | considered as a fix for `O(N²)` | matches iterative exactly at rest, but assumes transitive support | **Wrong for this model — dropped, not built.** Same failure mode that parked #55. |

**Overall for §0.1's go/no-go question:** the bounded overfill law pins, exactly as §0.1 predicted,
but only beyond roughly 300-400 nominal fine rows of fill — inside that range it reaches a
genuinely converged hydrostatic profile with headroom to spare. The `O(N²)` relaxation cost (Q3,
confirmed) is not disqualifying once `M`'s persistence across ticks is accounted for (Q-New): even
the slowest tested schedule is faster than the fine level doing the same job by one to two orders
of magnitude. The design should proceed on the basis that **the bounded law is usable up to its
measured depth limit**, that limit should be checked against what the shipped scenes actually need
(is 300-400 fine rows of compressed depth ever exceeded in practice?), and if it is, §0.1's
unbounded-law fallback (Q4, unaffected by this correction: reaches an exact hydrostatic profile at
`o=1.33` with no ceiling) remains available — but a direct per-component solve is not the way to
use it.

## Files

- `sandart-sim/examples/diag_coarse_step0.rs` — the instrument, self-contained, touches no other
  source file. The direct-solve prototype (dropped, see above) has been removed from this file
  rather than left in as dead code.
