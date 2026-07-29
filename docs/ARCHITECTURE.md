# Architecture / Orientation

This is the map. It exists because three separate confusions — each true-sounding, each
false or misleading — cost real time in one working session, and none of them were
written down anywhere before now. If you are new to this repo, read this document before
touching `physics.rs`.

Where this document and the code disagree, the code is right — this file describes intent
and invariants, not a spec the code is graded against. Function, type and constant names
are given so you can grep for the current reality rather than trust a line number, which
will have rotted by the time you read this.

## 1. Crate layout and responsibilities

The workspace (`Cargo.toml`) has five members:

- **`sandart-sim`** — the simulation engine. Owns the heightmap (`DrawingSimulation` in
  `sandart-sim/src/lib.rs`), the physics solver (`sandart-sim/src/physics.rs`, by far the
  largest file in the repo), the block-level-of-detail scheduler, the row-mass/quantile
  cache (`sandart-sim/src/quantiles.rs`), and the container-shape geometry
  (`eval_sandbox_shape` in `physics.rs`). No graphics API, no windowing, no UI. This crate
  also still carries the original marble-drawing-in-sand mode (displacement CA,
  `displace_line`) alongside the newer gravity/liquid flow solver — both live in
  `physics.rs`, selected by `SimulatorMode` (`Sandbox` vs `SandFall`).
- **`sandart-render`** — the WGPU rendering pipeline (`HeightmapRenderer` in
  `sandart-render/src/lib.rs`) and the fragment/vertex shader (`shader.wgsl`). Owns GPU
  resources: heightmap texture, colormap texture, shape-mask texture, lighting/camera
  uniforms, and the draw call. It is a *consumer* of the sim's output, not an independent
  source of simulation state — see the geometry-ownership boundary below, which is the
  single most important fact in this section.
- **`sandart-pattern`** — mathematical pattern generation and playback for the marble/pen
  modes: Spirograph, Lissajous, Rose curves, Fermat spirals, Gosper/Sierpinski
  space-filling curves, and `.thr`/G-code file parsing (`PlaybackController`). Unrelated to
  the sand/water physics; used by both front ends for the drawing-pattern feature.
- **`sandart-wasm`** — the browser front end. Wraps `DrawingSimulation` and
  `HeightmapRenderer` behind a `wasm-bindgen` API (`sandart-wasm/src/lib.rs`) and ships a
  hand-written `web/index.html` + `web/demo.js` UI. Also owns `build.rs`, which stamps the
  crate with a short git SHA and build timestamp (`build_git_sha`/`build_timestamp_epoch`,
  surfaced in the page footer as "Build") so a stale cached bundle is detectable.
- **`sandart`** — the native desktop app (`egui`/`eframe`), in `sandart/src/app.rs` and
  `sandart/src/main.rs`. Also wraps `sandart-sim` + `sandart-render`, with its own
  hand-rolled control panel. `main.rs` imports `sandart_render` under the local alias
  `renderer` (`use sandart_render as renderer;`) — grep for `crate::renderer` in `app.rs`
  expecting to find a `renderer.rs` file and you will not find one; it is `sandart-render`.

### The geometry ownership boundary (read this if you remember nothing else)

**Container geometry lives only in `sandart-sim`.** `eval_sandbox_shape`
(`sandart-sim/src/physics.rs`) is the sole source of truth for what shape a cell belongs
to (Circle, Square, Oval, Hourglass, MultiStageHourglass, GaltonBoard, StaircaseCascade,
ProceduralFunnel, MultiNeckHourglass — see `SandboxShape` in `sandart-sim/src/lib.rs`).
`DrawingSimulation::generate_shape_mask` rasterises that function once (on shape/parameter
change, not per-frame) into a `Vec<u8>` mask using three values: `MASK_OUTSIDE` (0, wall),
`MASK_INSIDE` (1, playable interior), `MASK_BOUNDARY` (2, interior cell adjacent to a
wall — used for the LED/rim rendering).

`sandart-render` does not evaluate shape geometry at all. It receives the mask as an
`R8Uint` texture (`HeightmapRenderer::update_shape_mask` / `shape_mask_texture` in
`sandart-render/src/lib.rs`) and the shader samples it with a plain `textureLoad` at
`shape_mask_tex` (`sandart-render/src/shader.wgsl`). That is the entire contract between
the two crates for geometry: sim computes, renderer samples.

**There is no duplicated geometry to keep in sync**, and no need to touch `shader.wgsl`
when adding or changing a container shape — only `eval_sandbox_shape` and
`generate_shape_mask` matter. (This was asserted as a real risk once; it was wrong, and it
cost time. If you are tempted to write that warning into a task again, don't — verify
against `shader.wgsl` first.)

One loose end this leaves behind: the `Uniforms` struct in `shader.wgsl` still declares a
`sandbox_shape: u32` field, and `sandart-render/src/lib.rs` still sets it (`sandbox_shape:
0`) when constructing uniforms. Grep `shader.wgsl` for `sandbox_shape` and the *only* hit
is the declaration — nothing in the shader ever reads it. It is dead. Harmless, but if
you're hunting for "where does the shader decide the shape", it isn't there and never
was.

## 2. The two front ends, and which one matters for testing

There are two ways to run this simulator — the desktop `sandart` binary and the
`sandart-wasm` browser build — and they are not equally important for verification.

**The user tests exclusively on the GitHub Pages deployment**,
`tanaeem-moosa.github.io/sandart/`, built by `.github/workflows/deploy.yml` on every push
to `main`. That workflow runs `wasm-pack build sandart-wasm --target web`, copies
`sandart-wasm/web/*` plus the compiled `pkg/` into a `dist/` folder, and publishes it to
`gh-pages`. It never touches the `sandart` desktop crate at all — desktop-only changes
(anything only reachable through `sandart/src/app.rs` and never through
`sandart-wasm/src/lib.rs`) are **invisible** to the user's actual testing, however correct
they are on the native binary.

Practically: if a change needs to be seen by the user, it needs a code path through
`sandart-wasm`, not just `sandart`. New UI controls, new material options, new shape
options — all need wiring in `sandart-wasm/src/lib.rs` (the `#[wasm_bindgen]` methods) and
`sandart-wasm/web/{index.html,demo.js}`, not just the egui panel.

Because the deploy pipeline is push-to-main and caching is opaque, the page footer carries
a build stamp (see `sandart-wasm/build.rs` and `displayBuildStamp()` in `demo.js`) showing
the short git SHA and build wall-clock time the running bundle was actually compiled from.
If the user reports behaviour that doesn't match what you just shipped, check that stamp
before assuming your change didn't work — it may just be a stale cached bundle.

## 3. Material selection is index-free by design

A past UI rewrite changed a `<select>`'s option *values* rather than its label text, and
because materials used to be keyed by array index, that silently repointed every existing
selection at the wrong material. The fix, now load-bearing: `MaterialMode` has a stable
string id per variant (`MaterialMode::as_str` / `MaterialMode::from_str` in
`sandart-sim/src/lib.rs`), `MaterialMode::ALL` is the single enumeration order, and the web
UI populates `<select id="material-1">` etc. at startup from
`WasmSimulationState::list_materials()` — a comment in `index.html` explicitly says not to
hand-add `<option>` entries there, because that duplication is exactly what broke
selection before. `sandart-sim`'s own test suite guards the round-trip
(`test_material_mode_string_ids_round_trip` in `sandart-sim/src/lib.rs`): every id must
round-trip through `from_str`/`as_str` and `ALL` must list each variant exactly once, with
no gaps or duplicates.

If you add a material, add it to `MaterialMode::ALL` and give it an id in `as_str`/
`from_str`; nothing else needs to change to make it appear correctly in the UI.

## 4. The physics model

The simulation is a conservative **edge-flux solver**: rather than updating each cell's
height directly, `settle_tick` (`sandart-sim/src/physics.rs`) integrates a velocity per
*edge* between two cells and moves a flux across that edge, debiting one endpoint and
crediting the other by the same amount. The core of this is `flux_edge`:

```text
yielded = sign(H_a - H_b) * max(|H_a - H_b| - tau, 0)   // tau = yield stress, 0 today
v_e    <- (v_e + c_sq * yielded) * damping               // per-edge momentum
flux    = clamp(v_e, -(donor b limits), +(donor a limits))
h_a -= flux ; h_b += flux
```

Two properties fall out of this that the older per-cell Laplacian update did not have:

1. **Mass conservation is structural, not incidental.** Every edge debits exactly what it
   credits, so the grid total cannot drift regardless of which blocks the LOD scheduler
   happened to run this tick — the old per-cell form only telescoped to zero if every cell
   updated in the same pass, and the LOD scheduler explicitly breaks that. Tests that check
   this (grep `rel_err` in `physics.rs`, e.g. `test_liquid_mass_conserved_under_gravity`,
   `test_sandbox_wave_conserves_mass`) measure relative mass error on the order of `1e-9`
   to `1e-8` — essentially float32 noise — and assert well under `1e-4`. Note the gap: the
   assertions are deliberately loose, but the *measured* band is six orders tighter, and it
   is the measured band that is the real invariant. Judge a change against ~`1e-8`, not
   against what the assertions happen to permit. A regression that
   breaks the driving/mass-limit split described below (section 4.2) shows up exactly as
   this number leaving that tight, noise-floor band.
2. **No unilateral clamp.** The old form ended in a bare `.clamp(0.0, 1.0)` on the updated
   height, which has no counterparty: flooring a negative excursion *adds* mass, capping at
   1.0 *discards* it. Here the donor's available mass and the acceptor's remaining capacity
   only ever *reduce a transfer*, never edit a height directly, so they cannot change the
   total.

`edge_sleeps` is the corresponding fast-path: a predicate that proves an edge would realise
*exactly* zero flux this tick (either both directions are constrained by donor-mass/
acceptor-room, or the edge is at equilibrium with nothing stored), so `flux_edge` and the
per-edge velocity write can be skipped entirely. It is what makes a settled pile or a
resting body of water cheap to simulate — only the free surface between "full" and "empty"
stays awake.

### 4.1 The unified gravitational head, `H = h + Phi`

`Phi(r) = -(g . r) * GRAVITY_HEAD_SCALE` is a positional potential added to each cell's
local fill to get its driving head `H`. `Phi` is exactly zero when gravity is
out-of-plane (Sandbox mode), so `H = h` and the solver degenerates to a pure free-surface
wave; it is a linear ramp along `g` in Sand-fall mode, so a downhill edge picks up
`|g| * GRAVITY_HEAD_SCALE` on top of the fill difference. The gravity slider therefore
moves behaviour *continuously* between the two regimes rather than switching between two
solvers.

`GRAVITY_HEAD_SCALE` (currently `25.0`) is tuned so that at the shipped Sand-fall gravity
(`0.04`), the head drop across one row of travel equals exactly one saturated cell of
fill — the natural unit that makes "fall into the empty cell below" outrank "spread into
the empty cell beside" by precisely one cell's worth of material.

`LATERAL_PRESSURE_SCALE` (currently `5.0`) is a second, narrower fix: `Phi` only depends on
an edge's two endpoints, so it correctly makes a column push down, but a *lateral* edge
under vertical gravity has no `Phi` term at all and drives purely on local fill
difference — which saturates at `cell_capacity` (~1.0), so a cell at the bottom of a
20-deep column and a cell under a single resting cell present an identical driving head to
their lateral neighbour once both are full. `column_depth` (computed top-down in
`settle_tick`, persisted tick-to-tick like `edge_vel_h`/`edge_vel_v`) estimates how much
resting mass sits above a cell in its connected static column, and `LATERAL_PRESSURE_SCALE`
weights how much that overburden adds to the lateral driving head. The constant was swept
empirically against `test_liquid_flowing_liquid_does_not_stand_in_walls` and
`test_liquid_stream_stays_coherent`; see the doc comment on the constant itself in
`physics.rs` for the full sweep data — it records a genuinely non-monotonic, noisy plateau,
not a clean optimum, so don't expect to "improve" it by re-sweeping without also re-reading
why the old sweep's shape couldn't be trusted (a phantom-depth bug at the source, since
fixed — see section 6).

### 4.2 The invariant that keeps conservation exact

`column_depth`, `GRAVITY_HEAD_SCALE`, and `LATERAL_PRESSURE_SCALE` feed **only** the
driving term (`head_a`/`head_b` passed into `flux_edge`) — **never** the donor-mass or
acceptor-capacity limits (`avail_a`/`avail_b`/`cap_a`/`cap_b`) that clamp the realised
flux. This split is what makes conservation exact rather than approximate: the mass limits
stay in raw mass units always, so a transfer can never exceed what a cell actually holds or
has room for, no matter how large or wrong the driving head's positional/pressure terms
get. If a change ever lets a positional or pressure term leak into a mass limit, the
symptom is not a crash — it is `mass_rel_err` (however a given test computes it) quietly
leaving its usual `1e-9`-to-`1e-8` band. Because the assertions are much looser than that,
such a regression can pass the whole suite green; it has to be caught by reading the
number, not by waiting for a failure. That is the single invariant most worth protecting
in this file.

## 5. `h` is one quantity; what changes is whether it carries a head

`h` (a cell's stored value in `heightmap.data`) is always the same thing: the amount of
material in that cell. It is **not** two different quantities depending on mode. What
changes between Sandbox and Sand-fall is whether `h` happens to be *aligned with gravity*,
and therefore whether it carries a hydrostatic head by itself.

- **Sandbox mode** (`gravity_dir` ~ zero, viewed top-down): gravity points straight out of
  the screen, perpendicular to the grid plane. `h` *is* the vertical extent of the column
  under that point, so `h` itself is the hydrostatic head — `h_a - h_b` between two cells
  is a genuine pressure difference, which is exactly why the `g = 0` liquid branch can
  drive directly on the raw fill difference.
- **Sand-fall mode** (gravity in-plane, viewed from the side): gravity now runs along the
  row axis, in the same plane the grid represents. `h` is no longer a length measured along
  gravity — it only says how full that pixel is, out of `cell_capacity_for(wetness)`. Fill
  alone carries no head in this orientation; the head has to come from *position* instead,
  which is exactly what `Phi` supplies.

This one idea (h stopped being a length along gravity when the view rotated) is the
explanation for three things that otherwise look like unrelated, arbitrary design
decisions:

1. **`GRAVITY_HEAD_SCALE` exists at all** to convert a fill quantity into head units —
   needed precisely because `h` is no longer a length once gravity is in-plane.
2. **The gravity-aligned edge normalises its driving term to `h / cap`** rather than raw
   `h` (see the comment at the `head_a = h_a / cap_a + gravity_dir.y * GRAVITY_HEAD_SCALE`
   line in `settle_tick`'s phase-0 branch). Without this, a head (`Phi`, tuned to cancel
   one *liquid* cell's fill per row) was being compared against a raw fill whose
   saturation point is 1.5 for granular material but only 1.0 for liquid — the mismatch bit
   dry sand (which climbed into empty air above a resting, at-capacity slab under low
   gravity) and never water. This was a real shipped bug, not a hypothetical; the
   normalisation is an exact no-op for liquid (`cap_a == cap_b == 1.0`, so `h / cap == h`)
   and only changes behaviour for granular material, which is the point.
3. **`column_depth` exists** because the side view has already discarded the information a
   top-down view got for free: a saturated cell's `h` saturates at `cell_capacity` no
   matter how deep the resting stack above it actually is, so "how much is really piled up
   here" has to be recovered by a separate top-down estimator (section 4.1) rather than
   read directly off `h`.

If you find yourself reasoning about `heightmap.data` and the reasoning depends on which
mode you're in, stop and ask whether you actually mean "does `h` carry a head here" — that
is almost always the real question, and getting it backwards has been the single most
expensive category of confusion in this codebase's history.

## 6. Jacobi vs. Gauss-Seidel driving — read the frozen-snapshot rule before touching an edge

Both the `g = 0` liquid branch and the gravity-active lateral edge in `settle_tick` drive
their edge velocity from `heightmap.data` — the tick's frozen starting heights — rather
than from `temp_heights`, the buffer being mutated in place as the sweep progresses. This
is not a style choice; it is a stability requirement, and the reasoning is written out at
length in `physics.rs` next to the `wetness >= 0.75 && !gravity_active` branch (search for
"Jacobi driving").

Driving off the live buffer makes the update Gauss-Seidel with a sweep direction that
alternates by tick/row, and Gauss-Seidel on this wave equation is not merely less accurate
than the frozen-snapshot (Jacobi) form — it is a *gain*. The linearised 1-D chain at
Water's `(c_sq, damping) = (0.24, 0.98)` measures a per-tick spectral radius of `1.20` for
the live-buffer (swept) form against `0.994` for the frozen form: the sweep injects ~20%
amplitude per tick while damping only removes ~2%, so a ripple grows until it hits the cell
cap and sticks there instead of decaying. Raising the cap doesn't fix it — it only moves
the ceiling and exposes the directional bias as asymmetry between left/right or up/down
flow.

The rule that generalises: **the driving term must read the frozen per-tick snapshot; the
donor-mass/acceptor-room clamps inside `flux_edge` must read the live buffer** (they need
to see what other edges incident on the same cells have already taken this pass, or a cell
could be drained twice over in one tick). Getting this backwards for either side breaks a
different thing — swap the driving term to live and you get the gain above; swap the mass
clamp to frozen and you can double-spend a cell's mass within a single tick.

As of this writing, the gravity-active lateral edge in `settle_tick` (the branch building
`head_a`/`head_b_full` from what the surrounding comments call `h_a_frozen`/`h_b_frozen`)
is under active work to bring it in line with this rule — **do not treat its current state
as settled**; read the comments immediately above that branch and check what `h_a_frozen`/
`h_b_frozen` are actually assigned from before relying on it. The g=0 branch's application
of the rule is the one to treat as the reference implementation.

## 7. `tau` (yield stress) — implemented, wired, and never turned on

`flux_edge` and `edge_sleeps` both take a `tau` parameter, fully implemented per the
formula in section 4: it clips the driving head before it can produce any yield/motion,
which is exactly the constitutive definition of a yield-stress (Bingham-like) material —
below `tau` nothing moves regardless of driving pressure, above it the excess drives flow.
`edge_sleeps`' second sleep condition (`|H_a - H_b| <= tau` with no stored velocity) is
explicitly documented as "the branch that would carry a granular material's whole settled
heap once `tau` is its yield stress rather than zero."

**Every call site in the current code passes `tau = 0.0`.** Grep `flux_edge(` and
`edge_sleeps(` in `physics.rs` — every production call hardcodes the literal. A granular
material is, physically, a fluid with non-zero yield stress, and the angle of repose is
exactly what a non-zero `tau` would express: the slope at which the driving head no longer
exceeds the yield threshold on any edge. This machinery has never been switched on. It is
the single most load-bearing unexploited piece of this codebase — granular repose behaviour
today comes entirely from the older, separate CA path (`add_sand_with_limit_properties`
and friends) and the donor/acceptor mass clamp, not from `tau`, even though the flux solver
already has a slot built for it.

## 8. External mass exchange (`apply_external_mass`)

`Heightmap::apply_external_mass` and the `external_mass_this_tick` buffer
(`sandart-sim/src/grid.rs`) exist because `column_depth`'s `resting_above` estimator
(section 4.1) has no way to see mass that was written into `heightmap.data` from outside
the flux solver's own edges — e.g. a continuous pour/tap in a test, or any future feature
that injects mass directly. Before this existed, an always-full source cell read as a few
cells of phantom "resting" depth every tick (nothing ever arrived via `edge_vel_v`, so
`in_transit_at` saw zero inflow even though the cell was, in fact, continuously fed), and a
width-1.5 deadband constant used to exist purely to swallow that phantom without also
swallowing genuine shallow overburden. `apply_external_mass` records the *entire* resulting
height into `external_mass_this_tick`, and `resting_above`'s computation subtracts it the
same way it subtracts `in_transit_at`'s estimate — eliminating the phantom at its source,
so the old deadband was deleted outright (recoverable from git history if a regression ever
needs it back).

The buffer is **signed**, with negative values **reserved for a future drain/sink** — a
waterfall-style feature that removes mass rather than adds it. Nothing has designed what a
negative value should mean for `resting_above` yet; the current code neutralises it with
`.max(0.0)` rather than letting an undesigned meaning through silently. If you're
implementing the drain, that `.max(0.0)` is exactly where its semantics need to be decided
first.

## 9. Block-LOD scheduling

The grid is divided into fixed-size blocks (`block_size`, currently 16, giving 32×32
blocks over the 512 grid). Each tick, `settle_tick` decides which blocks actually need to
run based on `last_displacements` and marks the outcome in `active_blocks: Vec<BlockActivity>`
(`Fast`/`Medium`/`Slow`/`Inactive`) — a fresh, this-tick-only snapshot, reset to `Inactive`
at the start of every call. `activate_neighbor` is the wake mechanism: any edge that
actually moves mass marks both its endpoints' blocks for re-simulation next tick, so a
sleeping block only re-wakes because something adjacent to it moved, never spontaneously.

This scheduler is itself a stopgap; `docs/future_adaptive_budget_simulation.md` proposes
replacing the tier system with a priority-budget scheduler and is, as of this writing,
still accurately marked "Approved Design / Not Started" — the 3-tier `BlockActivity` system
described here is still what ships.

**Gotcha:** `refresh_row_mass_active` (`sandart-sim/src/quantiles.rs`), the cheap
incremental refresh for the Sand-fall quantile-line overlay's per-row mass cache, only
re-sums rows belonging to a block-row that has at least one block active *in the tick it is
called on*. It is called from `refresh_quantiles_partial`
(`sandart-sim/src/lib.rs`), which itself only runs **every 5th tick** (`tick_count % 5 ==
0`), gated on `has_active`. Because `active_blocks` is a single-tick snapshot rather than
an OR accumulated across the skipped ticks, a block that was active on ticks N+1..N+4 (and
genuinely changed that row's mass) but has gone quiet again by tick N+5 — the one tick this
mechanism actually samples — will not have its row re-summed on that pass. The row's cached
mass can go stale exactly in the drain-then-sleep case the scheduler is designed to reward.
This is worth treating as a real, currently-unverified suspicion rather than a settled
finding — it was noticed while reading the code for this document, not chased down with a
repro. If you touch the quantile overlay and see mass lines that look like they've stopped
tracking a recently-settled region, this is the first place to look.

## 10. Development workflow

**The host has cargo but no linker.** Anything that actually links — `cargo build`,
`cargo test`, `cargo run` — must run inside the `sandart-dev` distrobox container:

```bash
distrobox enter sandart-dev -- /home/deck/.cargo/bin/cargo test --release -p sandart-sim
```

`sandart-wasm/build.rs` is written defensively around this: it needs `git` to stamp a
build SHA, but `sandart-dev` doesn't have `git` installed, so the build script treats every
git-command failure as an honest "unknown" fallback rather than failing the build — see the
comments in `build.rs` for why the `rerun-if-changed` strategy is a sentinel path that can
never exist, rather than watching `.git/HEAD`/`.git/logs/HEAD` (which would work in an
environment with git present, but silently register zero watch paths in one without it).

Deployment is automatic: pushing to `main` triggers `.github/workflows/deploy.yml`, which
builds `sandart-wasm` with `wasm-pack` and publishes `dist/` (a merge of `sandart-wasm/web/`
and the compiled `pkg/`) to the `gh-pages` branch, which GitHub Pages serves. There is no
manual deploy step and no staging environment — a push to `main` is a push to what the user
tests against.

## 11. Test methodology

Every bug of consequence found in this project so far has been a **small per-tick error**,
invisible to a test that only checks a total or a fully-settled end state. Three separate
liquid defects hid behind exactly that shape: something conserved mass in aggregate (or
looked fine once everything stopped moving) while still being wrong every single tick along
the way — directional bias, a stream that fans out sideways when it should stay narrow, a
sweep-order gain that only shows up while a ripple is still oscillating.

The methodology this has forced, and that new tests should follow:

- **Measure the flowing state, not just the totals or the settled state.** Look at
  `test_liquid_stream_stays_coherent` and `test_falling_stream_no_block_boundary_density_spikes`
  in `physics.rs` for the shape: they assert bounds on stream width, void counts, or
  per-tick behaviour *while material is actively moving*, not just mass-in equals mass-out.
  A test that only checks final state can pass while the transient behaviour is badly wrong.
- **Guard against a scenario going quiescent too early.** `physics.rs` has at least one test
  that explicitly asserts a minimum `total_flow` before trusting its other assertions
  ("only {total_flow} units of volume moved — the scenario went quiescent and this test
  would pass vacuously") — if nothing much moved, the test's other checks aren't testing
  anything. Any new flowing-state test should consider the same guard.
- **Verify every new test fails when its fix is reverted.** This is not optional
  process-following: doing exactly this caught two silently vacuous tests in this
  project's history — tests that passed both before and after the fix they were meant to
  guard, because they weren't actually exercising the changed code path. A test that has
  never been seen to fail has not been shown to test anything.
- **`edge_sleeps` has its own instrumentation for this reason.** Sleeping is deliberately
  exact — the edges it skips would have moved zero mass — so a solver that silently stopped
  sleeping (i.e. paid full cost everywhere) would still pass every mass/behavioural test in
  the file while being 2.7x more expensive per tick. `edge_sleep_stats` (test-only,
  thread-local, in `physics.rs`) exists purely to make that mechanism observable, since its
  correctness has no other externally visible trace.
