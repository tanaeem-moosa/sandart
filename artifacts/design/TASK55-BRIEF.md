# Task #55 — brief (rewritten 2026-08-07, MAX-PROPAGATION redesign)

Supersedes every earlier TASK55-*.md. Those remain as evidence; this is the plan.

## THE GOAL (user's words)

- "arch fixes itself even if there is outflow at the bottom of the arch. not arch
  can't happen."
- Liquid levels fast, including while draining. Cave pockets equalise.
- A siphon over a barrier in a cave must work, and must be EMERGENT — nobody
  codes a siphon.
- The SAME approach handles solids, with different propagation and damping:
  "obviously no one expects u shapes to equalize for sand. but probably move a
  little, right?"

## REPO STATE

- `origin/main` = `6cbdb5ed`, DEPLOYED (gh-pages built from it). Contains: #61
  U-tube vessel, PAUSE + STEP controls, the static isolation spec, the first
  `compute_head_field`, and the pressure heat-map SOURCE switch.
- Local-only commit `d0754af3`: head-driven transport WIP. Default off. Blocked.
- UNCOMMITTED in the tree: the incremental-field redesign (persistent
  `head_field` on `DrawingSimulation`, `advance_head_field`, spec settle loop),
  plus a tick-cap reduction (`HEAD_FIELD_SPEC_TICK_FACTOR` 5000 -> 8).

Nothing since `6cbdb5ed` is deployed. The app is unaffected by all of it.

## THE NEW DESIGN: MAX-PROPAGATION, NOT AVERAGING

This is the user's formulation and it is the correct one.

### Why the current field is slow

`advance_head_field` relaxes each cell toward the AVERAGE of its wet neighbours
(Gauss-Seidel / SOR). Averaging is a DIFFUSION process, and diffusion settles in
`O(N^2)` sweeps. Measured: the static specs need ~10^5 ticks at w=512 to reach
their tolerance. Give it a realistic budget (~10^3) and it is tens of
reference-rows off.

A MAX update is not diffusion. It is Bellman-Ford, and it terminates in
GRAPH-DIAMETER sweeps — `O(N)`, not `O(N^2)`. At 512 cells that is ~500 sweeps
against ~250,000. Three orders of magnitude, from changing the operator, not from
tuning anything.

### The rule

The user's three rules ("two cells side by side take the max of both"; "a cell
below is max of self and the one above plus one"; "the one on top is max of self
and below minus one") collapse into ONE rule once expressed in head, because
`head = z + p` already carries the elevation term:

    head[i] = max( own_local_hydrostatic[i], max over connected wet neighbours head[j] )

No `+1`/`-1` anywhere. Head is what is constant through a body at rest, so
neighbours compare directly. This also removes the units trap that cost this
session two wrong attempts: there is no mass sum in it, only elevations.

`own_local_hydrostatic[i]` is the free-surface ELEVATION of the cell's own
contiguous wet column (`own_elev` of the topmost wet cell in that run, resetting
at every air gap).

### TWO SUBTLETIES, both load-bearing

1. **"Max of self" must mean the cell's own local hydrostatic value, NOT its
   previous tick's value.** Max is monotone: including history makes the field
   ratchet upward, and it can never come back DOWN when the source drains.
   Re-seeding from the own-column surface each tick is what lets it fall.

2. **Pinned cells must NOT take the max.** If every cell took the max, head would
   be uniform, `grad(head)` would be zero, and nothing would ever move. It is
   exactly the gap between a pinned LOW free surface and the HIGH interior
   beneath it that drives flow — and that gap IS the siphon. Dirichlet pins stay
   Dirichlet.

### Why this makes the seed work

With the own-column hydrostatic seed the field already holds the exact answer for
a plain resting column. Max-propagation then only has to carry the DIFFERENTIAL —
the lateral Pascal part, which is the piece `column_depth` structurally cannot
supply. That is a small correction over a short path, not a whole field rebuild.

## WHAT IS ALREADY PROVEN WORKING — DO NOT REBUILD

### Free fall carries no pressure (both specs PASS, residual exactly 0)

Two mechanisms, and the second is the actual guard:

1. **Transitive support**: bottom-up per column,
   `effective_support[i] = min(support_fraction(i), effective_support[below])`,
   plain `support_fraction(i)` where below is out of mask. MIN, NOT PRODUCT — a
   product decays through a tall resting column and would report its top as
   unsupported. One unsupported cell zeroes the whole column above it, so a
   falling body reads unsupported throughout, not just at its bottom row.
2. **Dirichlet pinning**: unsupported cells are pinned to `p = 0` and are WRITTEN
   every sweep rather than reading their neighbours. A pinned cell is a fixed
   boundary condition, not a receiver, so pressure cannot leak in from adjacent
   supported material however many sweeps run.

### The Dirichlet pins themselves

- Exposed top face (nothing wet+in-mask above): `p = 0` there, so pin to
  `own_elev = z + heights * depth_scale`. Own weight still bears on own bottom
  face.
- Nothing supporting from below: `p = 0` at the bottom face DIRECTLY, so pin to
  `z` alone.
- Same structural condition (pressure at an exposed face is zero), DIFFERENT
  numeric targets, because `p` is always measured at a cell's BOTTOM face.
  Pinning both alike leaks exactly one cell of weight.
- A SOLID roof (`MASK_OUTSIDE`) above is NOT an exposed top — pressed against a
  wall, not open to atmosphere. This distinction is the entire content of
  `spec_pascal_under_a_roof`.

### Other keepers

- `support_fraction` (#58) — shipped, read-only, never modify.
- The pressure heat-map SOURCE switch, and its `(tick_count, source)` upload
  cache with invalidation hung off `full_upload_needed`.
- PAUSE + STEP (Space / "."), JS-only.
- #61 `UTubeFlowThrough` and `physics::U_TUBE_RECTS`.

## SPEC HARNESS

`sandart-sim/src/task55_head_spec.rs`, a child module of `physics` via `#[path]`.
Simulates NO material: builds mask + heights, drives the field alone, asserts.
Scenarios are fractions of w/h, swept at w=64/128/256/512. Each spec returns
`Result<(), String>` with measured numbers in the `Err`; `#[ignore]`d wrapper
each, plus a NON-ignored scoreboard pinning the exact passing
`(spec, head-source)` pairs.

RATCHET, upward only. Never widen a tolerance, delete a spec, or ignore the
scoreboard to make it green.

Current standing (7 specs x 2 sources):

| spec | legacy column_depth | new field |
|---|---|---|
| uniform head in resting open column | PASS | FAIL (slow) |
| pressure linear in depth | PASS | FAIL (slow) |
| head field resolution-invariant | PASS | PASS |
| free fall has zero pressure | FAIL | **PASS** |
| free fall pressureless THROUGHOUT | FAIL | **PASS** |
| Pascal under a roof | FAIL | FAIL (slow) |
| head single-valued across a body | FAIL | FAIL (slow) |

All three new-field failures are ONE cause — slow propagation — not three bugs.
Max-propagation is the fix.

Still to spec once transport works: siphon over a barrier (emergent); draining
vessel surface dips over each outlet; sand U-shape relaxes partially and STOPS at
repose; sand pile holds repose; hourglass discharge independent of fill height
(ALREADY CORRECT ~1.01x — do-not-regress); wet-sand gradient interpolates
monotonically with no discontinuity.

## TRANSPORT (local commit d0754af3, still valid as a design)

A THIRD BRANCH inside the existing flux-edge solver's driving-head expression
(physics.rs ~5274 lateral, ~4779 vertical) — NOT a new mass-moving pass. Reusing
that solver inherits mass conservation by construction, the donor/acceptor clamps,
and per-edge momentum damping (the velocity bound). LIQUID ONLY (both endpoints
liquidity >= 0.999): the field has no yield criterion, and a repose slope is a
permanent surface gradient that must produce ZERO flow.

CANARY: `test_dry_sand_has_angle_of_repose` with the toggle forced ON. The refuted
multiplicative head flattened 19.29deg -> 0.41deg.

Three dynamic specs written: falling water must not drift sideways; draining
vessel surface must dip toward its outlet; per-tick displacement bounded by a CFL
argument. `test_task55_dynamic_transport_spec_scoreboard` is `#[ignore]`d.

## USER DESIGN DECISIONS TO HONOUR

- **Pressure does NOT depend on material flow.** Hydrostatic pressure is a
  function of geometry alone — depth below the connected free surface, plus
  support. No flow history, no divergence source term. An earlier suggestion of
  mine to add `dp/dt = -c^2 div(u)` was numerical machinery for reaching the same
  answer iteratively, not physics, and is DROPPED.
- **Pressure must rise going down and fall going up, while siphons still work.**
  This is automatic because we relax HEAD, not pressure: `p = head - z`. If we
  relaxed pressure toward neighbours we would get pressure constant everywhere,
  which is exactly wrong.
- **Pressure should propagate FASTER than material.** Do not slow material to
  achieve this — material is already CFL-bounded at <= 1 cell/tick and slowing it
  would just look sluggish. The ratio is set by sweeps-per-tick, which is cheap
  to raise (scalar ops, no clamping, no capacity checks).
- **Reduce the baseline material transfer so pressure has headroom to increase
  it.** Right now baseline flow runs at essentially its cap, so a pressure
  gradient can only ever subtract. This is the same defect as #60 from the other
  side (`VERTICAL_PRESSURE_CAP_MULT` clamps the vertical head under one cell of
  depth, making #54's "deep material falls faster" inert). Do this AFTER the field
  is right — tuning against a wrong field is wasted.
- **Test pressure propagation WITHOUT material first.** The static spec harness is
  exactly that. Do not judge the field through end-to-end levelling metrics.

## MISTAKES MADE THIS SESSION — do not repeat

- **A post-relaxation floor fights the solver.** Applying an overburden floor
  AFTER the sweeps overwrites their answer and cannot be corrected. Measured
  worse than no floor.
- **`column_depth` is a MASS sum; head is an ELEVATION.** They coincide only when
  every cell is exactly full, and `cell_capacity_for` allows more than 1.0 per
  cell — a settled cell holds ~1.23. Seeding head with accumulated mass drove
  `spec_pressure_is_linear_in_depth` to slope 1.234 instead of 1.0. This is the
  same units trap as the earlier `eta = h + column_depth` bug. The seed must be an
  ELEVATION.
- **A seed changes the PATH to steady state, not the destination.** The spec
  harness drives to steady state, so a seed alone cannot move a settled residual.
  Only a change to the fixed point (or to the operator) can.
- **I twice mis-attributed a residual change.** The "0.76 -> 47.9" swing was the
  TICK CAP (640,000 -> ~1,000 ticks), not the floor. Re-measure with one variable
  at a time.
- **The deleted union-find coarse jump only ever fired when every Dirichlet pin in
  a component agreed on one value** — i.e. the trivial case. Every interesting
  configuration (a draining vessel whose surface dips toward its outlet) has
  disagreeing pins by construction. It accelerated only the case that needed no
  acceleration, and it HID the `O(N^2)` settling problem rather than solving it.
  Do not reintroduce it.
- **`debug_assert!` is compiled out in release.** A convergence guard written that
  way let the browser silently ship a barely-relaxed field. Measured on a live
  U-tube at w=512: mean overlay brightness 199.9/255 for `column_depth` against
  47.3/255 for the head field — the user reported "no heatmap shows".
- **A huge retry cap turns a correctness failure into a slow test suite.**
  `HEAD_FIELD_SPEC_TICK_FACTOR = 5000` gave a 640,000-tick cap at w=512 and took
  the lib suite from 72s to 484s. Caps should fail fast.

## BUILD ORDER

1. Swap the averaging update in `advance_head_field` for the MAX update. Seed each
   wet cell from its own column's free-surface elevation each tick. Leave the
   Dirichlet pins alone. Measure ticks-to-settle per spec, per resolution, before
   and after.
2. Get all 7 static specs passing at their existing tolerances. NO transport work
   until they do.
3. Verify the pressure heat-map's new-field source now renders comparably to
   `column_depth` (previous measurement: 199.9 vs 47.3 mean brightness over
   visible cells at w=512). Push, and have the user look at the U-tube and the
   procedural cave with PAUSE on.
4. Only then wire transport (d0754af3's design), with the repose canary.
5. Then reduce baseline material transfer so pressure has headroom (#60, #54).
6. Levelling diagnostics become an OUTCOME check, never primary evidence.

## SOLIDS: ONE CODE PATH, parameters interpolated by liquidity

REQUIRED, not stylistic — a wet-sand gradient can only behave continuously if one
solver spans both ends. Two solvers with a blend will show the seam.

| | liquid | dry granular |
|---|---|---|
| propagation | fast, isotropic, no attenuation | slower, ATTENUATES (Janssen: force chains shed load to walls) |
| anisotropy | none | vertical vs lateral differ (earth-pressure K) |
| yield | none | Mohr-Coulomb: NO flow below the critical slope; RIGID, not slow |
| damping | low | high |

Follow the existing `k_of_liquidity` idiom. Wet sand is the continuum, never a
third case.

KNOWN LIMIT already in code: the transitive-support pass is per-COLUMN and
captures no LATERAL support. Material on a ledge, or held in an arch, is supported
by something not directly beneath it. That is force-chain mechanics and belongs to
the solids half.

## WHY THE FIELD DOES NOT REUSE THE SCHEDULER'S SUPPORT PREDICATE (asked, answered)

`fresh_overburden_must_blocks` (physics.rs:1001) answers a DIFFERENT question —
"might anything in this 32x32 block move?", a boolean, `unsupported AND
has_room_to_move` — and gets transitivity free over TIME via
`activate_neighbor_upstream` (physics.rs:3896), one block per tick. A one-tick lag
is harmless for scheduling and fatal for a field read within-tick. The error
asymmetry is opposite too: the scheduler is built to OVER-include (a false
"unsupported" just wastes compute), while the field cannot be wrong at all. The
PRIMITIVE `support_fraction` IS shared.

FOLLOW-UP worth considering once the field is cheap: have the scheduler read the
field instead of maintaining its own predicate.

## DO NOT RETRY

- **Multiplicative free-surface head** (`flux ~ conveyance(depth) * grad(eta)`):
  destroys the angle of repose (19.29deg -> 0.41deg, non-vacuity anchor 0.00 for
  both DrySand and Water) and was SLOWER at w=512.
- **Global relaxation of HEIGHTS** (the shipped-then-refuted
  `elliptic_liquid_level_pass`): teleports water, levels free-falling material
  sideways, flattens draining surfaces. It made the fast global operation move
  MASS. Propagate the FIELD globally; move MASS locally.
- **HashMap component grouping as the cost driver**: profiled at 0.02% of runtime.
- **The union-find coarse jump** (see mistakes above).

## ENVIRONMENT AND STANDING RULES

- **HOST HAS NO LINKER.** Everything that compiles or runs goes through
  `distrobox enter sandart-dev -- bash -lc "cd /home/deck/projects/sandart && ..."`.
  No git and no jj inside that container.
- `cargo check -p sandart-wasm` TYPECHECKS NOTHING (the crate is
  `#![cfg(target_arch = "wasm32")]`-gated). Always
  `--target wasm32-unknown-unknown --release`.
- Integration tests do NOT run in the main test command — the intentional lib
  failure short-circuits it. Run separately: `perfect_simulation_determinism`,
  `fresh_pressure_field_toggle`, `pressure_heatmap_head_field_toggle`,
  `head_field_transport_toggle`.
- Deploy artifact: `wasm-pack build sandart-wasm --target web`. CI builds it on
  push to main; `deploy.yml` now also has `workflow_dispatch` for manual re-runs.
- **A PUSH IS NOT A DEPLOY.** Confirm the run exists before saying "deployed":
  `curl -s https://api.github.com/repos/tanaeem-moosa/sandart/commits/<sha>/check-runs`
  (`total_count: 0` means nothing fired). `git log -1 origin/gh-pages` names the
  source commit. `gh` is NOT installed; the unauthenticated API works because the
  repo is public.
- `test_water_blob_stays_left_right_symmetric_under_gravity` FAILS INTENTIONALLY
  as a marker of a known unfixed bug. NEVER fix, weaken, `#[ignore]`, retune,
  retitle it, or change its scan order or tolerances. A run with only that failing
  is GREEN.
- Never weaken any test to make a change pass. Never tune a constant to land
  inside a passing window.
- Do not change `block_size` or the 32x32 tiling (sandart-sim/src/lib.rs:444-451).
- Do not modify `support_fraction`, `recompute_column_depth`, `pressure_project`,
  `clamp_edge_feasible`, `fresh_overburden_must_blocks`,
  `elliptic_liquid_level_pass`.
- Do not alter web colour-scheme `<option>` VALUES; do not hand-add `<option>` to
  the material select (populated from `list_materials()`).
- `syncSettings()` pushes the whole panel on every control change — nothing on
  that path may reset the sim (bug of this kind fixed once in `b28ff5a`).
- `shader.wgsl` compiles at RUNTIME; a WGSL error is a blank canvas, not a build
  failure.
- w=512 is the primary resolution; 64/128/256 are diagnostic instruments.
- No browser driver exists. The user is the only visual instrument, via the
  gh-pages deployment. Never claim visual verification.
- Delegate implementation to Sonnet subagents; keep verification in the main
  thread. Be concise in replies; detail goes in the ticket.
