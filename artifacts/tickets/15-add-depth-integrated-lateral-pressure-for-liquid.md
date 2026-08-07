# #15 — Add depth-integrated lateral pressure for liquid

**Status:** completed

---

NEXT UP. Agreed order: #15 -> #19 (Stage B) -> #12 (visual polish) -> #17.
User asked to try this one with a SONNET agent.

STARTING STATE (verified 2026-07-28): main = 1dc3e38a, working copy clean, everything pushed.
sandart-sim 69 passed / 0 failed / 4 ignored, ~9.2s. Whole workspace green.
bench_sandfall @ budget 1024: Water ~1.21 ms/tick, DrySand ~8.54 ms/tick.

=== THE ARCHITECTURAL GAP ===

In Sand-fall the grid is a vertical cross-section and h is a FILL FRACTION, so the lateral
driving head between two cells is h_a - h_b. Between two FULL cells that is identically zero.
A 20-deep column of water therefore exerts exactly the same lateral push as a 1-deep one. Real
hydrostatic pressure grows with depth below the free surface. This is not a tuning issue.

This is why the water-walls fix (#13, commit db623d05) was only partial. Measured headroom:
  current in_transit fix: 400-tick enclosed-void integral 38437 -> 30060 (~22% better),
    stream coherence bit-identical (max_width 8, peak_h 1.0000)
  deleting the in-transit limit entirely: 38437 -> 6304 (-84%)
    BUT the falling stream fans from 8 cells to 59, i.e. total dispersion
So ~3/4 of the available improvement is NOT reachable through avail_*.

Also contributing: phase 0 sweeps bottom-to-top Gauss-Seidel, so every cell in a draining
column looks locally free-falling (room_below ~ 0.16, outflow ~ inflow ~ 0.83) even when it is
part of a 6-wide, 20-row sheet that is macroscopically supported.

=== TWO CANDIDATE FIXES ===
1. Transitive support propagated up each column from the floor — a cell is supported if the
   cell BELOW is supported, not merely if it is full. Distinguishes a resting sheet from a
   falling stream regardless of local fill.
2. Depth-integrated lateral head — drive lateral flux by depth below the free surface rather
   than by fill-fraction difference. Physically correct; makes deep water spread harder than
   shallow water, which is the actual missing behaviour.

=== THE REAL CONSTRAINT ===
Both are in direct tension with test_liquid_stream_stays_coherent (falling 4-cell stream must
stay <= 8 wide, peak fill >= 0.5). Falling water must stay narrow while supported water
spreads. Any fix has to separate those two cases on something OTHER than local fill, which is
exactly what neither currently does. If both cannot be satisfied, STOP AND REPORT the tension
with numbers rather than weakening either assertion.

=== MUST NOT REGRESS (all recent, all hard-won) ===
- Wave stability: the g=0 liquid branch drives edge velocities from heightmap.data, a frozen
  per-tick snapshot. Gauss-Seidel on a wave equation is a GAIN (spectral radius 1.23/tick vs
  0.994). Do not tidy it back to temp_heights. Guarded by the four test_sandbox_wave_* tests.
- Wave propagation: wake magnitude is the head difference across owned edges, NOT an absolute
  level, and MUST_SIMULATE_THRESHOLD is 1e-4 with no Sandbox/gravity split. Reverting either
  half either stalls waves or makes the domain permanently hot. Guarded by
  test_sandbox_wave_reach_is_budget_independent and test_settled_sandbox_pool_does_not_stay_hot.
- Edge sleeping + the granular_share cell early-out. Guarded by test_edge_sleeps_predicate and
  test_settled_liquid_sleeps_and_wakes (asserts sleep FRACTIONS — sleeping leaves no trace in
  any heightmap or mass total, so deleting it would pass every other test).
- Colour: u8 storage + stochastic rounding, tolerance 0.005 set from a 36-realization sigma
  study (0.001 was ~1.4 sigma and would flake). Guarded by
  test_color_boundary_does_not_diffuse_under_gravity.
- Fragile set: test_serpentine_no_sand_leaking (lib.rs, mass_err < 1e-4, tightest in suite),
  test_no_floating_sand_under_gravity, test_residual_sand_drains_to_zero,
  test_material_presets_and_avalanche (flow > 0.0 after a SINGLE tick, all 14 materials),
  test_no_mass_leaks_into_out_of_mask_cells, test_liquid_stream_stays_coherent,
  test_liquid_flowing_liquid_does_not_stand_in_walls.
- Granular tripwire: test_liquid_has_no_angle_of_repose (--ignored) prints DrySand spread,
  currently exactly 6.

=== TEST GAP TO CLOSE ===
Existing liquid tests measure SETTLED states or mass totals. That blind spot has now hidden
three separate defects (wave instability, water walls, wave stall). Any new test here must
measure the FLOWING state.
