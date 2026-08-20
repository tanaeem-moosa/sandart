# Coarse-level debug overlay

Built in response to: *"It would be nice if I could see the coarse sim on an overlay."* Two
independent Debug-group checkboxes, off by default, tinting the same 64x64 coarse tile grid the
LOD block heat-map already uses. Nothing here has been visually verified — there is no working
browser driver in this environment. Everything below is reasoned from the code and the field
values, not from a screenshot.

---

## 1. Which quantities, and why both

`CoarseState` (`sandart-sim/src/coarse.rs`) carries several fields. Two of them answer the
question the user actually needs answered — *why isn't the coarse level moving material as fast as
it evidently could* — and they answer two different halves of it, so both are exposed rather than
picking one:

- **`eta`** — coarse hydraulic head, the driving potential. Flat across a connected body at rest.
  This is the instrument for "does the coarse level's own picture of the world look sane" — is a
  settled pool actually flat, do the U-tube's two arms visibly equalise, is there structure at all
  or is it noise.
- **`delta`** (`M - A`, coarse-fine disagreement) — "work the fine level has not done yet", and the
  signal a future scheduler will clock blocks on (build step 4, not yet built). This is the
  instrument for "where is the coarse and fine level disagreeing, and in which direction".

`m_mass`/`a_mass` were not exposed: they're mass, not potential or disagreement, and both
questions above are already covered by `eta` and `delta` without them. Offering a selector would
have meant either a third `<option>`-driven control (fighting the existing pattern, which the brief
explicitly said not to fight) or cramming two unrelated colour scales onto one texture; two plain
checkboxes were cheap and let both be viewed at once, exactly like the block heat-map and pressure
heat-map already can be.

## 2. Wiring, file by file

**`sandart-sim/src/lib.rs`** (the only file touched in the crate `coarse.rs`/`physics.rs` are
off-limits in — this is a `DrawingSimulation` accessor, same pattern as the existing
`block_heat_texels`/`pressure_field_texels`):

- `coarse_eta_texels(&self) -> Vec<u8>` — row-major `COARSE_GRID * COARSE_GRID` (64x64) bytes.
  **Superseded design note**: this originally used per-frame min/max normalisation (stretch
  whatever spread is on screen to fill the ramp). That was wrong and has been replaced — see
  "Scaling, corrected" below; the rest of this bullet describes the current behaviour.
  Scaled against a FIXED physical reference, `base_head = GRAVITY_HEAD_SCALE * gravity_dir.y.abs()`
  ("one row of gravity head" — the exact quantity `coarse.rs` nets out of `phi` to build `eta`, and
  the exact scale `physics::coarse_delta_eta` drives fine edges with), re-centred each frame on the
  `inside` tiles' own MEAN (not min/max — see below for why that's not the same mistake). Falls
  back to `1.0` (the value `base_head` takes at the shipped default Sand-fall gravity 0.04) when
  `gravity_dir.y` is ~0 (Sandbox mode). Non-inside tiles (dry land, the exterior, or the whole grid
  when `coarse.available` is false at grid ≤ 64) are 0 — the same "off/no-data" convention the
  other two overlays already use for their sequential ramps.
- `coarse_eta_stats(&self) -> Option<(f32, f32, f32, f32)>` (new) — `(min, max, mean, reference)`
  over `inside` tiles, `None` if nothing coarse-coupled is on screen. The SAME computation
  `coarse_eta_texels` uses internally, exposed so the numeric readout (§3a) can never drift from
  what the colour ramp actually encoded.
- `coarse_delta_texels(&self) -> Vec<u8>` — same shape and sizing. Symmetric around a fixed
  physical zero, `norm = 0.5 + 0.5 * clamp(delta[C] / capacity[C], -1, 1)` — **per-TILE fixed
  reference** (`capacity[C]`, that tile's own nominal fill capacity — `M` and `A` are each
  individually bounded near it, so `Delta` naturally ranges `-capacity[C] .. +capacity[C]`), not a
  per-frame `max(|delta|)` (also superseded — see below). `capacity[C]` is geometry, fixed except
  when the shape rebuilds, so this is NOT frame-dependent rescaling: a genuinely tiny disagreement
  reads near-grey regardless of any other tile or frame. 0.5 is *always* exactly "no disagreement".
  Non-inside tiles map to **128 (neutral), not 0** — 0 is a real, strongly-coloured endpoint on a
  diverging ramp (maximally negative), so mapping "no data" there would paint dry land the same hue
  as the tile most starved by the coarse level.
- `coarse_delta_max_abs(&self) -> Option<f32>` (new) — plain `max(|Delta|)` over `inside` tiles, in
  raw mass units, for the numeric readout (§3a) only; not used by the texel encoder above, which
  normalises per-tile rather than against one scene-wide number.
- All four guard on buffer lengths matching `COARSE_GRID * COARSE_GRID` and degrade gracefully
  (all-zero/all-128/`None`) otherwise, matching `update_coarse_eta`/`update_coarse_delta`'s
  no-bounds-check upload contract exactly.
- Doc comment on all four notes the staleness contract: `coarse_state.tick()` (in `update()`, which
  populates `eta`/`delta`) only runs while `coarse_pressure_coupling` is on, so with that toggle
  off this overlay shows whatever the buffers last held — a property of that toggle, not a bug in
  this one.

### Scaling, corrected (coordinator review)

The first version of this overlay used **per-frame min/max normalisation** for `eta` and per-frame
`max(|delta|)` for `delta`. Both were wrong for the same reason: they stretch whatever spread is on
screen right now to fill the whole colour ramp, so a field spanning `0.0001` and one spanning
`10.0` render **identically** — and worse, a nearly-flat field renders as dramatic, fully-saturated
structure that is pure amplified noise. That is exactly backwards for an instrument whose primary
job is to answer *"does the coarse `eta` field have a real gradient at all, or is it nearly flat"* —
the one distinction per-frame normalisation is structurally incapable of showing.

Both are now scaled against a **fixed physical reference** instead:

- `eta`: `base_head` — one row of gravity head, the literal quantity already used to net elevation
  out of `phi` and to drive fine edges. The mapping still **re-centres each frame on the tiles'
  own mean** — this is not a relapse into the same mistake, because centring only moves the
  *origin* (removing `eta`'s arbitrary, physically-unanchored system-wide offset) while the *gain*
  stays fixed at `base_head` regardless of what's on screen. A deviation of a given size in
  `base_head` units always produces the same colour shift, frame to frame and scene to scene.
- `delta`: each tile's own `capacity[C]` — fixed geometry, not data.

Neither is a frame-dependent rescale of the kind that broke the diagnostic; both are a fixed unit
converted into an origin-appropriate reading.

### Numeric readouts

Per the coordinator's second suggestion, both overlays also get a plain-number readout in the
console footer, gated on their checkbox, refreshed on the same once-per-second cadence as the
saturation-decile legend: `eta` shows spread (`max - min`) **expressed in `base_head` "rows"**, so
`0.021` reads immediately as "nearly flat" and `1.480` as "over a full row of head between the
extremes" without the reader needing to separately know the scale; `delta` shows the raw
`max(|Delta|)` in mass units directly. This was added alongside the fixed-scale colour ramp, not
instead of it — the colour still shows *where* on the grid the extremes sit, the number removes any
remaining ambiguity about *how big* they are.

**`sandart-render/src/lib.rs`**:

- Two new textures, `coarse_eta_texture` / `coarse_delta_texture`, IDENTICAL shape to
  `block_heat_texture` — fixed `HEAT_GRID_SIZE x HEAT_GRID_SIZE` (64x64) R8Unorm — not
  `pressure_heat_texture`'s `grid_size`-scaled shape, since the coarse grid IS the 64x64 LOD block
  grid.
- Two new bind group layout entries (bindings 8, 9) and bind group entries, mirroring binding 6/7.
- `LightingUniforms::coarse_eta_enabled` / `coarse_delta_enabled`: u32 flags that **repurpose the
  two remaining `_pad_heatmap` slots** rather than growing the struct — `size_of::<LightingUniforms>()
  == 240` compile-time assert is untouched, no layout change.
- `update_coarse_eta` / `update_coarse_delta`: byte-for-byte the same body as `update_block_heat`
  (same row-alignment-safety branch, dead at 64 but kept for parity), each writing into its own
  texture. Same **no bounds check on `data`'s length** contract as `update_block_heat` — callers
  must upload exactly `HEAT_GRID_SIZE * HEAT_GRID_SIZE` bytes.
- Two test fixtures in this file that construct `LightingUniforms` (`test_headless_render_capture`
  and the marble test) updated from `_pad_heatmap: [0; 2]` to the two new named fields.

**`sandart-render/src/shader.wgsl`**:

- Two new `texture_2d<f32>` bindings, `coarse_eta_tex` (8) / `coarse_delta_tex` (9), read via
  `textureLoad` exactly like `block_heat_tex` (no sampler).
- `LightingUniforms.coarse_eta_enabled` / `coarse_delta_enabled` replace the two `_pad_heatmap0/1`
  scalars.
- Two new overlay blocks in `fs_main`, placed after the pressure heat-map block (same
  past-casing/past-marble placement contract), each gated on its own `_enabled != 0u` and blended
  at the same flat 0.55 as the other two overlays.

**Colour ramps** — see §3.

**`sandart-wasm/src/lib.rs`**:

- `coarse_eta_enabled: bool` / `coarse_delta_enabled: bool` fields on `WasmSimulationState`,
  initialised `false` (off by default, matching every other Debug overlay).
- `set_coarse_eta_overlay(&mut self, enabled: bool)` / `set_coarse_delta_overlay(...)` — plain
  field writes, no reset/reinitialisation path, same shape as `set_heatmap_overlay`.
- `render()`: both flags written into the `LightingUniforms` literal; both textures uploaded
  unconditionally each frame while their flag is on (no cache key like the pressure heat-map's —
  `coarse_eta_texels`/`coarse_delta_texels` are a single `O(4096)` pass over the coarse grid, same
  cost order as the always-uncached `block_heat_texels`, not the `O(grid_size^2)` per-cell scan
  that made the pressure heat-map's cache worth having).
- `get_coarse_eta_stats(&self) -> Vec<f32>` (new) — `[min, max, mean, base_head_reference]`, or
  empty when nothing coarse-coupled is on screen (same empty-when-unavailable convention
  `get_saturation_deciles` already uses). Thin wrapper over `sim.coarse_eta_stats()`. `mean` is
  carried through even though the JS reader's face/hover text doesn't use it, so the tuple stays a
  complete, self-describing snapshot of what the colour ramp itself was built from.
- `get_coarse_delta_max_abs(&self) -> Vec<f32>` (new) — `[max(|Delta|)]`, or empty likewise. Thin
  wrapper over `sim.coarse_delta_max_abs()`.

**`sandart-wasm/web/index.html`**:

- Two new checkboxes in the Debug group, `check-coarse-eta` and `check-coarse-delta`, both
  unchecked by default, with a comment explaining why both exist and what each answers.
- Two new console-footer readout rows, `coarse-eta-stat-row` / `coarse-delta-stat-row`, hidden by
  default (`display: none`), inserted into the same 3-column footer grid as the fps/ms/blocks
  stats — see §"Numeric readouts" above for what each shows.

**`sandart-wasm/web/demo.js`**:

- `syncSettings()` now calls `state.set_coarse_eta_overlay(document.getElementById('check-coarse-eta').checked)`
  and the `delta` equivalent.
- **Listeners confirmed present**: both
  `document.getElementById('check-coarse-eta').addEventListener('change', syncSettings)` and the
  `check-coarse-delta` equivalent are registered alongside the existing heat-map checkboxes'
  listeners — this is exactly the step `check-head-field-transport` missed, so it was checked
  explicitly rather than assumed.
- `updateCoarseOverlayStats()` (new, module scope like `updateSaturationDeciles()` — required by
  `scripts/check_js.js`, which exists because function-scoped helpers have shipped silently
  unreachable before): shows/hides each footer row per its checkbox, reads
  `get_coarse_eta_stats()`/`get_coarse_delta_max_abs()`, and writes the spread-in-rows / raw
  `max(|Delta|)` numbers described above, with the fuller numeric detail in each row's hover
  `title` (same "face vs. hover" split the fps/ms footer entries already use). Called from
  `syncSettings()` (so a just-toggled checkbox's row appears immediately, not after up to a second)
  and once per second from `tick()`'s existing stats block, right alongside
  `updateSaturationDeciles()`.

## 3. Colour ramps

`shader.wgsl` already has two sequential ramps, deliberately on different hue arcs so they're never
confusable mid-comparison:

- block heat-map: blue → teal → orange-red (~230° → 160° → 20°)
- pressure heat-map: violet → magenta → pale yellow (~270° → 320° → 50°)

**`eta` (sequential)** gets a third hue arc, dark forest green → mid green → pale warm cream
(~150° → 125° → 60°), so it's never confused with either existing overlay. Like both existing
ramps it moves in lightness AND hue together (dark/saturated → pale/warm) so the cold end reads
against pale sand and the hot end reads against the dark casing.

**`delta` is diverging, not sequential** — it's the first ramp in this shader with a real, fixed
zero, so it needed a different shape, not just a different hue. Vivid blue (`M < A`, coarse behind
fine) → **mid-grey** (agreement) → vivid orange (`M > A`, coarse ahead). The neutral sits at *mid*
lightness rather than white or black specifically because it has to hold contrast against pale sand
and dark casing simultaneously; an achromatic white-or-black zero would vanish into one of the two.

## 4. What this should show, reasoned from the field definitions (not observed)

The scaling fix's whole point is that magnitude is now legible against a fixed unit
(`base_head`/`capacity[C]`), not the frame's own spread — so, unlike the first version, a
genuinely-flat `eta` and a genuinely-sloped one are now expected to look *different*, not identical.
That distinction is the answer to the coordinator's direct question, so it's worth stating up front:

**Is a nearly-flat `eta` field now visually distinguishable from a sloped one? Yes, by
construction.** A field whose whole spread is a small fraction of one `base_head` (e.g. the settled
pool below) maps every `inside` tile to a value close to 0.5 on the ramp — near-uniform mid green,
regardless of how "zoomed in" that spread would have looked under the old per-frame stretch. A field
whose spread is comparable to or exceeds a full `base_head` (the U-tube below, mid-imbalance) pushes
tiles out toward the ramp's dark and pale ends, producing visible, structured colour variation. The
same absolute deviation always produces the same colour shift no matter what else is in the scene,
so "does the colour vary" is now a direct, non-misleading answer to "is there a real gradient" — and
the numeric readout (spread expressed in `base_head` rows) removes any residual doubt a colour
alone might leave, e.g. distinguishing "nearly flat, spread = 0.02 rows" from "flat-looking on this
small monitor but actually spread = 1.3 rows".

- **A settled pool**: `eta` is defined to be flat through a connected body at rest. Under the fixed
  scale, "flat" now means "spread small relative to one `base_head`" — the settled pool should
  render as a near-uniform mid green AND the eta-spread readout should sit near `0.00x` rows. If the
  pool instead shows visible gradient (colour clearly off mid-green) or the readout reports a
  spread on the order of `0.1` rows or more, that IS the defect the user is chasing — the coarse
  level's own picture isn't flat, so it can't be driving the fine level as hard as it should.
  Compare directly against HANDOVER.md §5.3's noted "coarse column not having earned hydrostatic
  compression" bias at resting seams (order ~1 `delta_eta` at a seam per §4.1's measurement, i.e.
  close to a full `base_head`) — if the readout lands near that order rather than near zero, this
  is that known residual, not a new defect. `delta` should be near-uniform grey there too (`M`
  should have converged to `A` at rest), with `max(|Delta|)` small relative to typical tile
  `capacity[C]` (tens of mass units at grid 512).
- **A draining hourglass**: `eta` should show a visible split between the upper and lower chamber
  (higher head above the neck than below, while material is falling) — under the fixed scale this
  reads as genuinely different colours (not just "different from the frame's own baseline"), and the
  eta-spread readout should sit comfortably above the settled-pool baseline, order `0.1`-`1`
  `base_head` rows depending on how much head the falling column is carrying. The neck itself is
  likely to read near the boundary between the two, and the neck's poor coarse conveyance
  (HANDOVER.md §4.2, `k` badly under-predicted at a tile-boundary neck) should show as an abrupt
  step rather than a smooth gradient across it. `delta` is the more diagnostic of the two here:
  watch for a persistent hot spot (either sign, now legible against `capacity[C]` rather than
  whatever the frame's own worst tile happened to be) right at the neck or in the chamber the
  stream is landing in — the design's dead-end list flags exactly that region as the common failure
  case for local deadband rules, so sustained, non-trivial `|Delta|/capacity` there (not transient
  noise) is a reasonable expectation.
- **A U-tube**: this is the clearest read of the two, and the clearest test of the fix. Right after
  a mass imbalance is introduced, `eta` should show the two arms as two visibly different,
  each-internally-uniform hues WITH a spread readout well above the settled-pool baseline — the
  imbalance is exactly the kind of real, large gradient the fixed scale is built to make legible.
  As the system equalises, both the colour difference between arms and the spread readout should
  shrink together, converging toward the settled-pool baseline (uniform hue, spread near `0.00x`
  rows) rather than an arbitrary per-frame "looks flat now" — "does it show the U-tube arms
  equalising" is answered by watching the readout trend toward zero, not just by eyeballing whether
  two hues look close. `delta` should spike (opposite sign in each arm) at the moment of imbalance
  and relax toward grey (and `max(|Delta|)` toward the settled-pool baseline) as `M` tracks `A`
  back down; a `delta` that stays hot in one arm — a `max(|Delta|)` readout that plateaus well above
  the settled baseline — long after `eta`'s spread readout has dropped near zero would be the direct,
  numeric version of HANDOVER.md's open question about why coarse-driven transport isn't moving
  material as fast as it evidently could.

## 5. One out-of-scope fix, forced

`sandart/src/app.rs` (the desktop binary, outside the file ownership for this task) also
constructs a `LightingUniforms` literal and used `_pad_heatmap: [0; 2]`. Repurposing that field in
`sandart-render` broke its compile, so it was updated to the two new named flags (`0` for both —
same "no native desktop control for it" comment pattern already used there for
`heatmap_enabled`/`pressure_heatmap_enabled`). Two lines, mechanical, `cargo check -p sandart`
confirmed clean afterward.

## 6. Verification

- `cargo check -p sandart-sim --lib` — clean.
- `cargo check -p sandart-render` — clean.
- `cargo test -p sandart-render --release` — **shader compiles at runtime** (this is the guard
  against a silent blank-canvas WGSL error) and the headless render/bind-group-creation tests pass:
  2 passed, 1 ignored (pre-existing `#[ignore]`); `shader_wgsl_parses_and_validates` and
  `validator_rejects_broken_shaders` both pass.
- `cargo check -p sandart-wasm --target wasm32-unknown-unknown --release` — clean.
- `cargo test -p sandart-sim --lib --release` — **102 passed / 10 failed**, the same ten named
  pre-existing failures (`test_water_blob_stays_left_right_symmetric_under_gravity`,
  `test_dry_sand_has_angle_of_repose`, `test_head_field_transport_repose_non_regression`,
  `test_liquid_pool_levels_flat_in_closed_box`, `test_liquid_stream_stays_coherent`,
  `test_sandbox_wave_decays_to_flat_pool`, `test_sandbox_wave_reach_is_budget_independent`,
  `test_sandbox_wave_reflects_off_boundary`, `test_sandbox_wave_stays_left_right_symmetric`,
  `test_task55_dynamic_transport_spec_scoreboard`) — baseline held.
- All seven integration suites pass: `overfill_pressure_toggle`, `perfect_simulation_determinism`,
  `fresh_pressure_field_toggle`, `pressure_heatmap_head_field_toggle`,
  `head_field_transport_toggle`, `pressure_sensitive_flow_toggle`,
  `coarse_pressure_coupling_toggle`.
- `node scripts/check_js.js` — all checks pass.

**Re-verified after the scaling fix** (coordinator review, this section's numbers are from that
second pass): `cargo test -p sandart-render --release` — same result as above, shader still
compiles and validates at runtime, headless bind-group creation still succeeds (this change touched
no textures/bindings/uniforms, only the CPU-side byte encoding and two new plain wasm getters, so
`sandart-render`/`shader.wgsl` were untouched in this pass). `cargo check -p sandart-sim --lib`,
`cargo check -p sandart-wasm --target wasm32-unknown-unknown --release`, and
`cargo test -p sandart-sim --lib --release` (102/10, same ten names) all re-run clean.
`node scripts/check_js.js` re-run clean after adding `updateCoarseOverlayStats()`.

Nothing has been pushed or deployed as part of this change.
