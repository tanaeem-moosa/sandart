# Session handover — the mirror axis, red-black, and why water holds a 45-degree wall

Covers 2026-09-02 to 2026-09-09, against `13da08c`. Supersedes
`SESSION-HANDOVER-2026-08-30.md` for anything about asymmetry or levelling.

Read `artifacts/design/ASYMMETRY-2026-09-08.md` alongside this — it holds the measurements; this
holds the state and the next step.

---

## 0. STATE, FIRST

- Local `main` is `13da08c`. **`origin/main` is at `cc3fc3f` — one commit behind.**
  `13da08c` is a docs/comments-only commit with no behaviour change, which is why it was not
  pushed. Push it whenever; nothing is waiting on it.
- Library suite **99 passed / 4 failed**, and that is the expected state. Integration suites (all
  five), doctests, wasm32 check and `check_js` all pass. CLAUDE.md lists each failure and why.
- There is a pre-existing `stash@{0}` ("STAGE C ... parked, not for main yet") from an older
  session. It was NOT touched and is not part of any of this.
- Two stale agent worktrees still sit under `.claude/worktrees/` holding full tree copies.

The four failures:

1. `test_water_blob_stays_left_right_symmetric_under_gravity` — the sanctioned #56 marker. Must
   keep failing.
2. `test_sandbox_wave_stays_left_right_symmetric` — residual solver asymmetry, deliberately left
   visible. Tiny (4.4e-7) but no longer decays.
3. `test_liquid_flowing_liquid_does_not_stand_in_walls` — cost of red-black, deliberately shipped.
   **The user has explicitly said they do not mind this artifact** ("I don't mind the waterflow
   lines"), so do not spend effort on it unless asked.
4. `test_cascade_no_dam_or_neck_merge_across_chamber_count_range` — MultiStage/cascade geometry
   only, see §4.

---

## 1. What shipped

- **`b28f127`** — LOD block is a constant 8 cells at every resolution (`DEFAULT_BLOCK_SIZE`),
  replacing `block_size = grid/64`. No-op at grid 512. Budget throttles derived from block count.
  Also fixed `test_sandbox_wave_reach_is_budget_independent` by fixing the TEST: its bit-identical
  amplitude assertion contradicted what `budget_n` is for.
- **`bd1cd0a`** — `eval_sandbox_shape`'s mirror axis corrected from `w/2` to `(w-1)/2`. **Every
  vessel in the app had been asymmetric by construction, at every resolution.** Kills the
  persistent settled lean (~1% of mass, always the same direction).
- **`cc3fc3f`** — red-black edge colouring of the lateral pass. Seven of eight tick-phase
  mechanisms are now bit-identical no-ops. Mid-drain mirror error down 27-36%.
- **`13da08c`** — `diag_water_levelling`, plus removal of eight stale `pressure_project` /
  `pressure_gate` references.

**The user has seen `cc3fc3f` in the app and approved the symmetry** ("cc3 seems to be doing pretty
good job at better symmetry").

---

## 2. THE OPEN PROBLEM: water holds a ~45 degree wall

This is the live task. The user's framing, which is the spec:

> *"I don't need water to flatten fully, that is how we ended up on our misadventure with pressure,
> but flatenning more would be ideal. and not having the clear angles."*

So the target is **more flattening and softer angles**, explicitly NOT full levelling.

**The cause is incompressibility.** A full water cell has zero room, so lateral flow cannot pass
THROUGH it; a piled body levels at about one cell per tick, and a uniform transport limit is exactly
what produces a uniform straight facet. Shown in one line: refill the closed box to h=0.5 so every
cell has headroom and max surface slope goes 5 -> 2.

**It is not a red-black regression.** `diag_water_levelling` at t=3000: baseline range 141,
`cc3fc3f` 168. ~19% worse, but overwhelmingly pre-existing.

`diag_water_levelling` is the metric to judge any candidate against. Run it with:

    cargo test -p sandart-sim --lib --release -- --ignored --nocapture diag_water_levelling

---

## 3. THE NEXT STEP: solver sub-stepping

**Every other route measured is refuted** (§9 of ASYMMETRY-2026-09-08.md has the numbers):

- Raising water's `cell_capacity_for` — that is compressibility, not headroom. Fails
  `test_liquid_is_incompressible` and both task55 scoreboards. **The user spotted this
  independently before the suite did; it is a trap because it looks excellent on every other
  metric, including turning failure 3 above green.**
- A global preference for lateral flow via `in_transit_at`'s withholding — does what is asked
  (max slope 5 -> 2) but every reduction down to 0.9 fans a 4-cell stream out to 18-30 cells. It is
  also ALREADY implemented for the case it is right in: `in_transit_at` withholds only up to
  `downstream_route`, so a cell that cannot fall already has full lateral availability.
- `LATERAL_PRESSURE_SCALE` re-sweep (5/8/12/20) — flat-to-worse.
- The multiplicative lateral gate — rejected twice in TASK55-VERDICT.md / TASK55-RESOLUTION.md.
  Do not resurrect without reading both.

**Sub-stepping is the only remaining lever that changes no physics at all** — it runs more solver
ticks per rendered frame, so the surface gets further along before the user looks at it. It is the
`+-1.0` clamp lever from `HANDOVER.md` §10 and nobody has tried it.

Going in:

- Cost is linear in frame time. The user's screenshot showed **27.7 ms/frame at grid 512**, close to
  the 33 ms budget, so there is little headroom at 512 and real headroom at 256 and below — which is
  where the worst of the reported defect was.
- `HANDOVER.md` §10 names the alternative (multi-cell transport per tick). That one breaks the
  one-edge-one-cell assumption the frozen-Jacobi pass is built on, so it is a design conversation,
  not an implementation. Sub-stepping first tells us how much of the artifact is just tick budget.
- Judge it on `diag_water_levelling` AND `test_liquid_stream_stays_coherent` together. Everything
  refuted so far bought one at the cost of the other.

---

## 4. Smaller open items

- **`anti_merge_ceiling` (the cascade failure).** `bd1cd0a` moved every chamber centre by half a
  cell. `multistage_neck_half_width`'s floor was fixed for this (0.5 -> 1.0: on a half-integer axis
  a cell's `|dx|` is always 0.5, 1.5, ..., so `|dx| < 0.5` admits NO cell and the neck closed
  completely). `anti_merge_ceiling = (chamber_w / 2.0 - 0.5).max(0.5)` has the identical
  sensitivity and has NOT been re-derived, so at `w=64, chambers=11, neck_width=0.06` adjacent
  chambers merge into open space. **This is app-visible for MultiStageHourglass.** Hourglass and
  MultiNeckHourglass are unaffected. Do not "fix" it by reverting the axis.
- **Red-black perf is unmeasured.** July's `d6d843b` saw ~8% on an older solver. Worth an A/B before
  sub-stepping, since sub-stepping spends the same budget.
- **Dead render-side overlay plumbing** (four textures, bind groups, `shader.wgsl` branches) is
  unreachable but present. Removing it means editing bind-group indices, unverifiable without
  loading the page, so it should ride along with a deploy someone is already eyeballing.
- **An unidentified RNG path reaches the liquid solver.** `test_tick_phase_mechanism_isolation`
  shows `K_RNG_SEED` moving a water-only scenario, while that test's own docs claim the RNG "lives
  in the granular path a liquid scenario never reaches". It is NOT the dispersion term (that scales
  by `granular_share`, identically zero for water; setting `DISPERSION_TAU_FRAC = 0` leaves the
  drain metric bit-identical). Unresolved.

---

## 5. Two lessons this session paid for

**Tests here have twice DOCUMENTED A BUG AS A FACT rather than reporting it.**
`test_water_blob_stays_left_right_symmetric_under_gravity` measured the mask, correctly concluded
its axis was `w - x`, and wrote that into a comment as a property of the geometry — so every
asymmetry number quoted for weeks was measured against a half-cell-off axis.
`test_no_floating_sand_under_gravity` carried its own inline copy of the hourglass boundary, twice,
with a hardcoded `center_x = 32.0`. **If a test has its own copy of the shape math, that is the
bug. Fill from the mask.**

**Comments here describe deleted machinery as live.** This session proposed diagnosing
`pressure_project` before checking it exists — it was deleted in `3bb6533`. The user caught it.
Eight such references were cleaned in `13da08c`, and `new_with_size`'s claim about a load-bearing
heat-map texture upload was also stale. **Before building on a named function, grep for its
definition.**
