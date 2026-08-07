# #33 — 2.10 — Design: sideways movement for water and sand under gravity

**Status:** pending

---

DESIGN TASK, requested 2026-07-29. Lateral movement under gravity is the least coherent part of the simulation and several open bugs converge on it. Produce a design before writing code.

## The central contradiction to explain
Water under gravity currently does BOTH of these, which should not be possible under one consistent model:
- OVER-spreads on impact: `test_liquid_splashes_on_impact` (ignored) measures a symmetric 8-cell blob reaching width 59 in a 64-wide box within 10 ticks. Unchanged by the Jacobi fix (ff7a255b) — so it is not the Gauss-Seidel gain.
- UNDER-spreads against walls: the enclosed-void total is 17570, against a 6304 reference for removing the in-transit limiter entirely. Water visibly towers rather than levelling, which the user reported from the deployment.

Explaining why one model produces both is the core of this design.

## Constraints as they stand
- No cell may flow against gravity: `gravity_active && gravity_dot < -0.01` -> `continue`. Upward motion is impossible BY CONSTRUCTION, so a splash can never rise.
- No diagonal flow under gravity: `if gravity_active && ndy != 0.0 { continue; }`, added in Stage B (73cebd33). Suspected cause of the thin stranded edge in #30 — a cell whose only downhill neighbour is diagonal is permanently stuck, worst on curved walls like Circle's top cap.
- Sand's lateral movement is entirely on the legacy CA path (see #26 / Stage C). Water's is on the flux solver.
- `tau` (yield stress) is fully implemented in `flux_edge` and `edge_sleeps`; every call site passes 0.0. Angle of repose is exactly what `tau` expresses and it has never been switched on.
- The lateral driving head is `h + LATERAL_PRESSURE_SCALE * column_depth`, re-swept in #28 and found to have NO headroom: a flat noisy plateau across the whole valid window ~3.5-18, bounded by failure at both ends.

## Questions the design must answer
1. Why does water over-spread on impact AND under-spread against walls? Same term mis-scaled, or two different mechanisms?
2. What did banning diagonal flow under gravity actually buy — CFL, conservation, something else — and what does it cost? Can it be re-enabled, or replaced with something that does not strand cells?
3. Should lateral movement be momentum-carrying (edge velocity, like the vertical edge) or purely pressure-driven? Currently it is a mix and the reasoning is not written down.
4. For sand: is `tau` the right mechanism for lateral resistance, and how does that interact with the CA path it currently lives on (#26)?
5. Does the no-upward-motion constraint need to survive? It makes splashes physically impossible and may be load-bearing for stability.

## Must not regress (for whatever implementation follows)
- 79 tests; do NOT weaken assertions, report numbers.
- `test_liquid_stream_stays_coherent`: max_width <= 8, peak_h >= 0.5.
- `test_water_blob_stays_left_right_symmetric_under_gravity` — the new signed-asymmetry test.
- Mass exact; measured band is 1e-9..1e-8, NOT the 1e-4 the assertions permit.
- Sand bit-identity is already broken by anything touching the shared diagonal valve — that needs explicit visual scrutiny, not just tests.
