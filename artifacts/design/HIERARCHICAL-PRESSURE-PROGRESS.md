# Hierarchical pressure — progress handover, updated 2026-08-19

For whoever picks this up next. The design is `artifacts/design/HIERARCHICAL-PRESSURE.md`. **Read
this file before that one** — measurement has since corrected several of its numbers, the user has
settled three questions it left open, and it contradicts itself in one place that cost real time.

**Nothing is pushed.** `origin/main` is still `f73fe1d`. Nothing is deployed, so none of this has
been seen running by the only visual instrument there is (the user).

---

## 1. Where the build order stands

| step | what | status |
|---|---|---|
| 0 | Does the coarse level pin at `o_max`? | **DONE.** Pins past ~300-400 rows → use the unbounded law |
| 1 | Coarse geometry (`open_cells`, `capacity`, `k[e]`) | **DONE**, `e50c0bc` |
| — | LOD block → `grid/64` (user-directed, not in the original order) | **DONE**, `94d7390` |
| 2 | Restriction + instrumentation (`A`, `M`, `eta`, `Delta`) | **DONE**, `f670cd48` |
| 3 | Couple the coarse head into the fine solver | **DONE**, `f670cd48` + `409913bf` + `5893c6ae` |
| — | Debug toggle + ON/OFF A/B | in flight |
| 4 | Fine-level over/underclocking on `\|Delta\|` | **NOT STARTED** — the last unbuilt piece |
| 5 | Tune at scale | not started |

**Acceptance criteria** (design §8), all four required simultaneously:

| # | criterion | status |
|---|---|---|
| 1 | A pile of liquid settles fast enough to look right at 512 | **unverified** — needs the user's eye |
| 2 | The U-tube shows visible upward movement in the riser | **unverified** — needs the user's eye |
| 3 | No oscillation | **unverified** — churn measurement in flight |
| 4 | The hourglass does not break | **MET.** 0 of 9,883 falling cells carry pressure |

Criterion 4 is the one that killed every previous attempt at long-range liquid transport in this
project. It is the only one that can be settled headlessly, and it is settled.

---

## 2. The three user rulings, and why they matter

### 2.1 The LOD block IS the pressure tile — `block_size = grid_size / 64`

**Overrides the standing rule** in `artifacts/HANDOVER.md` §1 and `artifacts/design/HANDOFF.md`;
both are updated in place. The rule's *reason* survives and is worth keeping: block count must stay
resolution-invariant, because `sandart-render`'s `update_block_heat` uploads into a fixed
`HEAT_GRID_SIZE^2` texture **with no bounds check**.

Why: over/underclocking is the point, the block is the unit that gets clocked, and a per-block clock
rate derived from coarse-fine disagreement only exists if the block and the tile producing the
signal are the same region.

### 2.2 Fine is the source of truth; coarse buys rate, capped by what coarse says

User's words: *"fine and coarse each ground each other. fine obviously is the source of truth. but
coarse can be used to speed up fine as long as it doesn't exceed what coarse says."*

**This settles §6's I0**, which the design explicitly parked as "must be decided before anything is
built". The answer is that both couplings are used, in different roles:

- `eta` is the **driving potential** — this is what buys rate;
- `|Delta| = |M - A|` is the **flux budget** (I4), bounding the real mass the coarse term may cause
  to leave a tile in one tick.

### 2.3 The coarse level IS a 64x64 simulation, one step per tick

User's words: *"we only need one coarse simulation per tick"* and *"let's just make coarse sim same
is 64x64."*

Both were corrections of work already built, and both were right. "Adaptive" applies to the **fine**
level's over/underclocking, not to the coarse level's step count — an earlier agent (and this
session's main thread) misread it and built an adaptive sweep ceiling, since removed.

The coarse level's dynamics are now `physics::settle_tick` run over a 64x64 nested sandbox. Nothing
reimplements the overfill law or its constants. **This is what fixed acceptance criterion 4** — §4.1.

---

## 3. What is built

- `sandart-sim/src/coarse.rs` — `CoarseGeometry` (fixed 64x64, `t = grid/64`, per-edge conveyance
  `k[e]`) and `CoarseState` (restriction `A`, persistent `M`, anchor `lambda`, the nested 64x64 sim,
  `eta`, `P`, `Delta`).
- Coupling in `physics.rs`: inter-tile edges receive `delta_eta` in the driving head, in both
  passes. Real mass still moves only through fine edges.
- The flux budget (I4) and a derived deadband (I5) — `grounded[C] = support_mass[C] > 0`, an exact
  aggregate of real fine compression, no fitted epsilon anywhere in the decision.
- Exact incremental restriction, guarded by `incremental_restrict_is_bit_exact_to_full_rebuild`
  (bit-identity against a full rebuild, all ten shipped shapes, 300 ticks).
- A debug toggle, `coarse_pressure_coupling`, defaulting **ON**, wired sim → wasm → web with its own
  change listener.

**Verification baseline:** lib suite **102 passed / 10 failed** — the same ten named failures
documented in `HANDOVER.md` §10. All six integration suites pass, `perfect_simulation_determinism`
included, plus `sandart-render` and the wasm32 check.

---

## 4. Measurements that corrected the design

### 4.1 The over-drive, and what fixed it

The bespoke coarse level used `base_head_coarse = base_head * t` — eight rows of gravity per coarse
row at 512 — while the fine solver already applies one row per edge. A fine edge at a tile seam was
therefore driven at **1 + 8 = 9x gravity**.

The user found this by asking the right control question: *"falling sand works fine in 64x64
simulation today. why would it have issues in coarse simulation?"* It does not, once the coarse
level is actually a 64x64 simulation.

| | free-falling cells carrying pressure | `delta_eta` at resting seams |
|---|---|---|
| bespoke coarse, 16 sweeps/tick | 8.4% | ~8.0 |
| bespoke coarse, 1 sweep/tick | 16.9% | ~8.0 |
| **coarse = real 64x64 sim, 1 step** | **0.0%** (0 of 9,883) | 0.98 |

Cost fell with it: 30.1 → **23.4 ms/tick** at 512 Water, against a ~21 ms uncoupled baseline (~11%,
inside the design's 15% budget). **The "expensive but clean vs cheap but broken" tradeoff was an
artifact of the bespoke level and no longer exists.**

### 4.2 Numbers in the design that are wrong

- **§4's "5-cell neck gives `k = 5/8 = 0.625`" describes a number that does not exist.** The
  minimum-width neck row does not land on either row of the `t = 8` tile boundary at 512. Measured
  **`k = 0.375`**.
- **§4's `(t-1)/t` neck-inside-a-tile prediction over-predicts by 10-25 points**, but the worry
  holds: 0.729 (hourglass) / 0.762 (multistage) at 512 against a predicted 0.875. Roughly three
  quarters of necks vanish from the coarse model. **Necks are not handled.**
- **§2's "negligible" holds at 512 and NOT at 128**, where the block becomes 2x2: +50-65% ms/frame,
  MUST tier 22% → 67%.
- **§0.2 and §0.3 contradict each other** about what the fine cell reads — §0.2 says
  `eta[tile] - z_fine` (per fine cell), §0.3 says add `eta[tile]` (per tile, constant). The code
  follows §0.3, **and §0.3 is the correct one**: working §0.2's form through DOUBLES the residual
  rather than removing it, because the coupling term must be *equal* for vertically adjacent fine
  cells at rest, which is what a per-tile constant gives. This cost a wrong recommendation in this
  session; do not re-derive it.

### 4.3 Confirmed as predicted

- **The elevation double-count is real** — `P[D] - P[C]` measured 7.94-7.98 against a predicted
  `t * base_head = 8.0`.
- **Coarse relaxation is diffusion** — sweeps-to-settle 114/430/1598/5848 for chain length
  8/16/32/64, exponent 1.89. Affordable because `L` is capped at 64 and `M` persists.
- **The bounded law pins** past ~400 nominal rows: `max(o)` exactly `0.9000`, 12.9% of wet tiles
  pinned, interior residual stuck at exactly `base_head_coarse`. Hence the unbounded law.
- **The diagonal-neck hazard is a non-issue** — corner-touching cells give `k = 0` and are not in
  the same 4-connected component either.

---

## 5. Open problems, in priority order

1. **Criteria 1, 2 and 3 are unverified.** They need the deployment and the user's eye. This is the
   biggest gap between "works" and "done", and everything below is speculation until it is closed.
2. **Edge saturation.** §8's "no bang-bang transport" branch fires **311,196** times over 400
   hourglass ticks (34,782 U-tube). This matters structurally: I1's no-overshoot guarantee holds
   because the solver lands exactly on the equilibrium — **a saturated edge does not**, it moves its
   full mass limit instead. So the anti-ringing argument lapses precisely on those edges, and that
   is the pre-#70 failure mode. **Predicting this would collapse once the over-drive was fixed was
   WRONG**: hourglass went 298,055 → 311,196. It has a different cause, unknown.
3. **Residual ~2x gravity at resting seam edges** (`delta_eta ~= 1.0` where rest implies 0). Not a
   formulation error (§4.2) — it is the coarse column not having earned hydrostatic compression, so
   `eta` is not yet flat. A systematic bias that does **not** vanish at rest.
4. **Necks are not modelled** (§4.2).
5. **Wake propagation reaches half as many cells per tick** since the block resize, and
   `test_sandbox_wave_reach_is_budget_independent` cannot see it — that test hardcodes its own
   `block_size = 16`. A real propagation change with nothing instrumenting it.
6. **Grid 128 is 50-65% slower.** Accepted, not fixed.

---

## 6. Dead ends — do not re-propose

- **A direct per-connected-component `eta` solve.** Tempting: `eta` is constant through a resting
  connected body, so one monotone 1-D root find per component per tick gives the equilibrium exactly,
  no sweeps, no diffusion. **It is wrong.** A falling stream is mass-connected to the pool it lands
  in, so a component-wide `eta` hands the stream hydrostatic pressure it must not have — "support is
  not transitive", the failure that parked the #55 head field, and it breaks criterion 4.
- **Six deadband variants**, all measured worse than the shipped local-only rule (21% to 73.5%
  against 8.4%). Tabulated in `coarse.rs`; the common failure is that the tile above a compressed
  tile is very often the same tile a falling stream is landing in.
- **§0.2's per-fine-cell elevation subtraction** as a fix for the seam residual — doubles it (§4.2).
- **Folding the coarse term into `gravity_head`.** `cached_vertical_lut` is keyed on it and holds a
  SINGLE entry, so every inter-tile edge rebuilt a 4096-entry table of bisections: 7-8s → >12
  minutes on one integration suite. The coarse term is a separate `coarse_head` parameter and the
  LUT fast path is taken only when it is zero.
