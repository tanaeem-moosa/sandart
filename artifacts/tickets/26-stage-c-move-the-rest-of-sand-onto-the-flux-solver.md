# #26 — 2.3 — Stage C: move the rest of sand onto the flux solver

**Status:** pending

---

Stage B (commit 73cebd33) moved only the GRAVITY-ALIGNED (vertical) edge onto the edge-flux solver, for all materials, and won 2.56x on DrySand (8.76 -> 3.42 ms/tick). Stage C is everything else.

## DO 2.2 (#20) FIRST — this is a real ordering dependency, not a preference
The whole point of Stage C is giving `tau` a non-zero value on the LATERAL edge. `tau` is compared against `driving`, and the lateral `driving` term is currently a TWO-PART RECONSTRUCTION:

    h_a + LATERAL_PRESSURE_SCALE * max(column_depth - LATERAL_PRESSURE_DEPTH_FLOOR, 0)

The first part saturates at `cell_capacity`; the second exists only to carry back the depth information the first one loses. Set `tau` against that and the resulting ANGLE OF REPOSE becomes a function of `LATERAL_PRESSURE_DEPTH_FLOOR` — a constant #20 exists to change or delete. Tune repose first and it gets retuned twice, the second time against sand's look, which no test measures.

(Caveat worth verifying rather than assuming: the lateral pressure term is currently gated on `cell_liquidity > 0.0`, so it may not touch dry sand at all today. If Stage C makes granular flow share that lateral driving term — which is the natural way to do this — the coupling becomes real. Confirm which world you are in before deciding the order is safe to ignore.)

## Root context — `h` means two things
Under gravity `h` is CELL FILL, saturating at `cell_capacity_for(wetness)` (1.5 dry / 1.0 liquid). At g = 0 it is a COLUMN HEIGHT. `GRAVITY_HEAD_SCALE = 25.0` converts between them. Every liquid bug this session lived on that seam. Sand has the only `cap != 1.0`, so sand is the material that feels it — the gravity-boiling bug had threshold exactly `g < cap / GRAVITY_HEAD_SCALE` and never affected water at all. Expect the same asymmetry here.

## What is still on the legacy cellular-automaton path
Roughly lines 2106-2500 of sandart-sim/src/physics.rs, reached when `granular_share > 0`:
- the LATERAL (grid-x) edge under gravity
- ALL granular flow at g = 0 (sandbox mode — only LIQUID waves were moved to flux there)
- the avalanche collapse safety valve ("A." — spike prevention)
- the main slope-driven flow loop (PROP_THRESHOLD / PROP_FLOW_RATE / PROP_GRAIN_SIZE)
- lateral avalanche dispersion — what forms a natural tall hill on the bed
- stochastic sideways dispersion / splashing
- the stochastic locking-and-sliding state machine ("C.", writes `sliding[]`)
- (the marble distance search also lives in this loop but is not flow — it must survive somewhere)

`granular_share = 1.0 - cell_liquidity` scales every transfer in that block; the three scaling sites are ~lines 2243, 2450, and the `granular_share <= 0.0` early-out at ~2127.

## The key asset: `tau` is already implemented and has never been switched on
`flux_edge` (line ~393) and `edge_sleeps` (line ~508) BOTH implement a yield stress:

    let yielded = if driving > tau { driving - tau }
                  else if driving < -tau { driving + tau } else { 0.0 };
    ...
    v_e == 0.0 && driving.abs() <= tau

Every call site passes `tau = 0.0`. A granular material is precisely a fluid with a non-zero yield stress, and the angle of repose is exactly what `tau` expresses. The machinery exists; it has simply never been given a value.

Corollary worth holding onto: sand is currently special-cased in ~24 `liquidity` blend sites, its own `cell_capacity_for`, its own `GRANULAR_FALL_C_SQ` / `GRANULAR_FALL_DAMPING` (lines 1722-1726), AND a whole separate CA path. Those constants are not independent physics — they are stand-ins for the one parameter that was never switched on. The real prize is that "sand" stops being a branch and becomes a number.

## Why this is HARDER than Stage B — do not assume it will go the same way
Stage B was easy because the vertical edge had NO granular character to preserve: the gravity term swamped `threshold`, so there was never a real vertical yield stress and `tau = 0` was already correct there (the Stage B agent verified this). The lateral edge is the opposite — essentially 100% of what makes sand look like sand lives there: piles holding an angle, avalanches letting go, grainy scatter.

Repose -> `tau` should map cleanly and is expected to come out BETTER than the hand-tuned version. The stochastic dispersion and the sliding/locking state machine do not map cleanly at all — they are deliberate noise, not pressure-driven flow. They either survive as an overlay on top of the flux solver or must be re-derived. Decide that deliberately and say why.

## Payoff
- Closes the last perf gap: DrySand 3.42 vs Water 1.36 ms/tick, i.e. 2.5x, entirely because the CA path gets neither edge sleeping nor the `granular_share` early-out.
- Retires a whole second flow model.
- Replaces empirical repose constants with a physical yield stress.
- Makes g = 0 granular behave consistently with everything else.

## The risk
This is where the app's LOOK comes from. `tau` set wrong and sand looks like water — and unlike every bug fixed this session, that failure is a matter of taste and NO TEST WILL CATCH IT. Do this incrementally, one edge at a time, measuring after each. Strongly prefer a working partial change with numbers over a sweeping rewrite.

## Must not regress
- All 77 tests pass. Do NOT weaken or relax any assertion — report numbers instead.
- Sand must still hold an angle: `test_material_presets_and_avalanche`, `test_no_floating_sand_under_gravity`, `test_residual_sand_drains_to_zero`. Add a measured pile-angle check — "does it still pile" is currently only implied.
- Colour must not diffuse: `test_hourglass_color_and_property_conservation_under_gravity`, `test_color_boundary_does_not_diffuse_under_gravity`. The layered-sand look is the whole point of the app.
- Liquid untouched: `test_liquid_stream_stays_coherent` (max_width <= 8), the flowing-walls void total (21938 today, or whatever #20 leaves it at), sandbox wave decay/reach/reflection, `column_depth` and the lateral pressure work.
- Bench `mass_rel_err` must stay ~1e-9 for both materials at all three budgets — if it moves, flux antisymmetry is broken; stop.
- Report Water and DrySand ms/tick before -> after.

## Test methodology
Every bug in this project has been a small per-tick error invisible to tests that measure totals or settled states — three separate liquid defects hid behind exactly that. Any new test must measure the FLOWING state. Model: `test_liquid_flowing_liquid_does_not_stand_in_walls`.

Also: verify each new test FAILS when its fix is reverted. Doing that this session caught two tests of mine that were silently vacuous.
