# Hierarchical pressure — progress handover, 2026-08-18

Written mid-build, for whoever picks this up. The design is
`artifacts/design/HIERARCHICAL-PRESSURE.md`; this file records **what has been built, what has been
measured, which of that document's claims turned out to be wrong, and which of its open questions
the user has since settled.** Read the design's §9 build order alongside this — the step numbers
below are its step numbers.

**Nothing here is pushed.** `main` is a local bookmark; `origin/main` is still `f73fe1d`. Nothing is deployed, so nothing has been seen by the only visual instrument there is.

---

## 1. Where the build order stands

| step | what it is | status |
|---|---|---|
| 0 | Falsify §0.1 — does the coarse level pin at `o_max`? | **DONE.** Pins beyond ~300-400 rows; use the unbounded coarse law (§4.3) |
| 1 | Coarse geometry: `open_cells`, `capacity`, `k[e]`, connectivity test | **DONE, verified, committed `e50c0bc`** |
| — | LOD block resized to `grid/64` (not in the original build order; the user directed it) | **DONE, verified, committed `94d7390`** |
| 2 | Restriction + instrumentation (`A`, `M`, `P`/`eta`, `Delta`), no coupling | not started |
| 3 | Couple, liquid-only, small `lambda`, flux budget on, LUT hazard settled | not started |
| 4 | Map the joint `(N, lambda)` stability region | not started |
| 5 | Tune, then unblock granular (I7) | not started |

---

## 2. The two rulings that changed the design

Both came from the user during this session. Neither is in the design document; the document
contradicts the first one outright.

### 2.1 The LOD block IS the pressure tile — `block_size = grid_size / 64`

This **overrides the standing rule** in `artifacts/HANDOVER.md` §1 and `artifacts/design/HANDOFF.md`
("do not change `block_size` or the 32x32 block tiling"). Both files have been updated in place to
record it as superseded rather than left to mislead. The rule's *reason* survives and is now written
down where it can be found: the block count must stay resolution-invariant, because
`sandart-render`'s `update_block_heat` uploads into a fixed `HEAT_GRID_SIZE^2` texture **with no
bounds check**.

Why: over- and underclocking is the point of the whole exercise, the block is the unit that gets
clocked, and a per-block clock rate derived from coarse-fine disagreement only exists if the block
and the tile that produces the signal are the same region.

### 2.2 Fine is the source of truth; coarse buys rate, capped by what coarse says

The user's words: *"fine and coarse each ground each other. fine obviously is the source of truth.
but coarse can be used to speed up fine as long as it doesn't exceed what coarse says."*

**This settles §6's I0**, which the design left explicitly unresolved ("`P` and `Delta` are
different couplings and the design must pick one"). The answer is that it is both, in different
roles:

- the coarse level supplies the **driving potential** (`eta`, §0.2/§0.3) — this is what buys rate;
- `|Delta| = |M - A|` is the **flux budget** (I4) — the total real mass the coarse term may cause to
  leave a tile in one tick, so the fine level never moves more than the coarse level asked for.

The fine level remains the only thing that moves real mass (I1/I6, untouched).

A corollary the user also settled: **coarse propagation speed is not a worry.** The coarse grid is
fixed at 64x64, so the chain length is capped at 64, and `M` persists across ticks — convergence
does not have to happen inside one tick. 64 sweeps/tick over 4096 coarse cells costs about one fine
sweep at 512. §10's "coarse relaxation is diffusion" bullet is therefore **downgraded, not
dismissed**: it is real (measured exponent 1.89, §4 below) and it is affordable.

### 2.3 A tempting fix that is WRONG, recorded so it is not re-proposed

I proposed replacing the iterative coarse relax with a **direct per-connected-component `eta`
solve** — since `eta` is constant through a resting connected body, one monotone 1-D root find per
component per tick gives the equilibrium exactly, with no sweeps and no diffusion. It was
prototyped as a measurement and then cancelled.

**Do not build it.** A falling stream is mass-connected to the pool it lands in, so a
component-wide `eta` hands the stream hydrostatic pressure it must not have. That is "support is not
transitive" — the failure that parked the #55 head field — and it breaks the design's acceptance
criterion 4 (the hourglass). Relaxation propagates at a finite rate and only pressurises what is
actually compressed; that property is worth its cost.

---

## 3. What is built and verified

### 3.1 Step 1 — coarse geometry (`e50c0bc`)

`sandart-sim/src/coarse.rs`, a `CoarseGeometry` on a fixed 64x64 coarse grid (`t = grid/64`: 8 at
512, 4 at 256, 2 at 128), holding `open_cells`, `capacity`, `inside` per cell and `k_x`/`k_y`
conveyance per edge. Built from `shape_mask` + `cell_props`, rebuilt at the end of
`generate_shape_mask()` **and only there**, so it is exactly as fresh as `shape_mask`.

**Nothing reads it.** That is the point of the step: it is testable standalone, and the simulation
is bit-for-bit unchanged while it exists (`perfect_simulation_determinism` is the check).

At grid 64 the scheme degenerates (`t = 1`, the coarse grid IS the fine grid, and a coarse cell
would double-count its own overfill pressure against itself), so the geometry reports
`available = false` rather than flooring `t`. Note this is the **opposite** choice from the LOD
scheduler's (§3.2), deliberately — the scheduler has no such correctness constraint.

Full report: `artifacts/design/STEP1-GEOMETRY.md`.

### 3.2 The block resize (`94d7390`)

`block_size` `(grid_size/32).max(1)` -> `(grid_size/64).max(1)`, so 64x64 = 4096 blocks at every
resolution. Everything that was a block **count** moved 4x with it, or it would have become a
silently 4x tighter throttle: `budget_n` 256 -> 1024 (**two sites** — construction and `reset`),
`BUDGET_MIN` 32 -> 128, `BUDGET_STEP_DOWN` 4 -> 16, `BUDGET_STEP_UP` 1 -> 4.

Outside `sandart-sim`, the block heat-map overlay hardcoded `32` in three places — `HEAT_GRID_SIZE`
in `sandart-render/src/lib.rs`, and `* 32.0` / `clamp(..., 31)` in `shader.wgsl`. **`shader.wgsl`
compiles at runtime**, so a miss there is a wrong overlay or a blank canvas, never a build failure.
`cargo test -p sandart-render` is the guard.

The floor stays `.max(1)`, so grid 64 gets `block_size = 1`. Flooring at 2 was drafted and reversed:
it would have made grid 64 the only resolution with a different block count, reintroducing exactly
the silent-misread class described in §2.1.

Full report and all numbers: `artifacts/design/BLOCK-RESIZE.md`.

### 3.3 Verification state

Run in the main thread after each commit, not taken on a subagent's word:

```
cargo test -p sandart-sim --lib --release        # 98 passed / 10 failed / 46 ignored
```

The **ten failures are the documented set on `main`** (`HANDOVER.md` §10) and are unchanged in
count, name and character across both commits. Baseline before this session's work was 91/10; the
+7 are step 1's new tests.

All six integration suites pass, `perfect_simulation_determinism` included, plus
`cargo test -p sandart-render`, `cargo check -p sandart-wasm --target wasm32-unknown-unknown
--release`, and `node scripts/check_js.js`.

---

## 4. Measurements, and the design claims they corrected

### 4.1 Confirmed

- **The elevation double-count (§0.2) is real, almost exactly as predicted.** Measured
  `P[D] - P[C]` = 7.94..7.98 between vertically adjacent coarse tiles against a predicted
  `t * base_head = 8.0` — ratio 0.993 to 0.997 through the pressurised interior. **The `eta`
  reformulation is mandatory, not optional.** (This number does double duty: it *is* the vertical
  equilibrium condition being satisfied, so it is also evidence the coarse interior converged.)
- **Coarse relaxation is diffusion (§10).** Sweeps-to-settle 114 / 430 / 1598 / 5848 for chain
  length 8 / 16 / 32 / 64; fitted exponent **1.89**. Real, and affordable for the reason in §2.2.
- **The unbounded coarse law works, and §4.3 says it is the one to use.** It reaches an exact
  analytic hydrostatic profile at `o = 1.36`, well past the bounded law's `o_max = 0.90` ceiling —
  so the coarse level *can* represent depth the fine level structurally cannot.
- **The diagonal-neck hazard (§4) is a non-issue.** Two cells touching only at a corner give
  `k = 0` on both shared edges, and they are not in the same 4-connected flood-fill component
  either, so `k = 0` is correct. Checked, not assumed.
- **The staleness floor is resolution-invariant in cells (§7b).** 4.3x as many blocks forced per
  tick (23.8 -> 102.6, predicted ~4x), same work in cells (6,093 -> 6,566).

### 4.2 Design document numbers that turned out to be wrong

- **§4's "5-cell neck gives `k = 5/8 = 0.625`" is a number that does not exist.** The
  minimum-width neck row does not coincide with either row of the `t = 8` tile boundary at 512.
  **Measured `k = 0.375`.** Anyone tuning against 0.625 would be tuning against a fiction.
- **§4's neck-inside-a-tile prediction of `(t-1)/t` over-predicts by 10-25 points**, but the worry
  holds: measured 0.729 (hourglass) and 0.762 (multistage) at 512 against a predicted 0.875, and
  0.527 / 0.465 at 256 against 0.750. **Roughly three quarters of necks vanish from the coarse
  model at 512.** §4 says "do not treat necks as handled" and that stands.
- **§2's "negligible" was written for the classification loop, not the whole scheduler.** At 512 it
  holds (Water 20.3 -> 21.1 ms/frame, DrySand 13.2 -> 12.4, both noise). At **grid 128**, where the
  block becomes 2x2, ms/frame rose **50-65%** and the MUST tier's share roughly tripled
  (22% -> 67% for Water). Shipped — under 2 ms absolute, and 128 is a diagnostic resolution — but
  it is a cost the design did not distinguish.

### 4.3 Step 0 — DONE, and it decides the coarse pressure law

**The bounded law pins, as §0.1 predicted, and the pinning depth is the number this step existed to
produce.** Measured at 512 with `t = 8`, sweeping the pool's nominal fill depth:

| nominal fill | `max(o)` | tiles pinned | interior residual | verdict |
|---|---|---|---|---|
| 300 rows | 0.80 | 0 of 1860 | 5e-4 | converges, thin headroom against `o_max = 0.90` |
| 400 rows | **0.9000 exactly** | **240 (12.9%)** | **stuck at exactly 8.0** | pinned: `P` genuinely flat, the literal §0.1 signature |

A residual stuck at exactly `base_head_coarse = 8.0` is the pathology stated in words: the
compression can no longer grow, so `P[C] - P[D] = 0` and **the coarse driving force vanishes at
exactly the depth it exists to supply.**

**Recommendation: the coarse level uses the UNBOUNDED compression law** (§4.1's Q4 result — exact
analytic hydrostatic profile at `o = 1.36`, well past the bounded ceiling). A container at 512 that
is mostly full is ~450 fine rows, which is past the measured pinning depth, so the bounded law fails
in the production case rather than in a contrived one. `M` is not real mass and carries no
conservation obligation (I6), so the coarse level is free to use a law the fine level cannot.

Why 300 rows converged when a naive reading of the demand law says it should not: compression is
**self-limiting**. `125 * (o + o^2/0.9) = D` uses the *occupied* depth, and compressing the column
shortens it, which lowers the demand. That feedback is why the pinning depth is ~400 nominal rows
rather than ~300.

**The "boundary lock" is retracted.** Under the corrected residual (largest realised transfer, not
largest stress) the interior converges cleanly — residual 5e-4, `eta` flat to four decimals through
the whole wet column. The original worst edge was a free-surface/vacuum pair, which carries
unsatisfiable stress in the existing **fine** solver too, on every tick. That section of
`STEP0-MEASUREMENTS.md` has been corrected in place.

**Ticks-to-settle for a cold 64-cell chain**, with `M` persisting across ticks and no anchor:

| `N` sweeps/tick | ticks to settle | per-tick cost, in fine sweeps @512 |
|---|---|---|
| 8 | 900 | 0.125 |
| 16 | 450 | 0.25 |
| 32 | 225 | 0.50 |
| **64** | **113** | **1.00** |
| 128 | 57 | 2.00 |

Total work is **~constant across `N`** (~113 fine-sweep-equivalents): batching more sweeps per tick
redistributes the same work across fewer ticks, it does not reduce it. So `N` is a latency dial, not
an efficiency one, and the design's §2 speedup table should be read that way. At the design's
proposed `N = 8`, a cold chain takes 900 ticks — probably too slow to look right; `N = 64` costs one
fine sweep per tick and settles in 113.

Instrument: `sandart-sim/examples/diag_coarse_step0.rs`. Report:
`artifacts/design/STEP0-MEASUREMENTS.md` (corrected in place, not appended — the old Q1 verdict and
boundary-lock narrative are gone rather than contradicted).

---

## 5. What to do next

**Step 0 is done (§4.3); its output is "use the unbounded coarse law".** Next is step 2, which is
where the design's
remaining open questions get decided against numbers rather than argued:

- **The restriction cost (§5 step 1).** `A[C]` cannot piggyback on COLLECT, for four reasons the
  design lists. Decide explicitly: own pass over the grid (and cost it), or one-tick-stale `A` (and
  re-derive I1/I6 with the extra unit of lag).
- **The `|Delta|` distribution over blocks in a settling pile.** Unmeasured, and it decides whether
  `|Delta|`-driven sub-stepping is affordable at all: a thin front is cheap and self-financing, a
  whole pool wanting `n_b > 1` is not. §7b names this as the thing to measure at step 2.
- **`capacity[C]`'s refresh cadence.** `cell_capacity_for` depends on wetness, which advection
  changes continuously. The design says decide explicitly; step 1 built it so the capacity term can
  refresh independently of the geometry, but the cadence is still unchosen.

Then step 3, which must not begin until two hazards named in §8 are settled:

- **LUT thrashing.** `cached_vertical_lut` holds a *single* entry keyed on
  `(overfill_ratio, unit, tension, gravity_head)`. If the coarse term is folded into `gravity_head`,
  the key changes per edge and every change rebuilds a 4096-entry table. Either the coarse term
  enters somewhere the LUT is not keyed on, or the LUT needs a real cache.
- **The deadband (I5) must be derived, not fitted.** The measured margin is already thinner than the
  proposed constant (worst falling-dominated tile 0.9809 against a proposed `eps_dead ~= 0.02`), and
  the trend runs the wrong way as tiles shrink. Derive it from the free-fall condition; the mixed
  tile — part stream, part pool — is the case that decides it.

Standing throughout: liquid-only (I7 — the granular yield criterion derives from *fine* pressures
and adding `P` to the driving term without adding it to `normal_p` collapses dry sand's repose angle
at depth), and `test_dry_sand_has_angle_of_repose` run with the coupling forced ON as the
non-regression check.

---

## 6. Loose ends this session created

- **Wake propagation now reaches half as many cells per tick**, since
  `activate_neighbor_upstream`/`_side` wake one adjacent block and a block is half as wide.
  `test_sandbox_wave_reach_is_budget_independent` prints an unchanged 70/245 because **it hardcodes
  its own `block_size = 16`** and cannot see production's value. So this is a real propagation
  change with nothing in the suite standing behind it. If disturbances look slower to cross the
  domain, this is why.
- **Grid 128 is measurably slower** (§4.2). Accepted, not fixed.
- **Nothing is pushed and nothing is deployed.** The user tests via the GitHub Pages deployment and
  is the only visual instrument; none of this has been seen running.
