# #55 — 2.32 — REWRITE: unified hydraulic-head field for liquid AND solids. Shipped multigrid pass is visually refuted (see artifacts/design/TASK55-BRIEF.md)

**Status:** in_progress

---

2.32 — Unified hydraulic-head field for liquid AND solids.

STATUS 2026-08-07: the FIELD is done and deployed. TRANSPORT is wired, correct on every spec, and still DEFAULT OFF pending #64.

## Shipped and live (origin/main = 2af44fcc, deployed and verified green)

- `task55_head_field::advance_head_field` computes head by MAX-PROPAGATION, not averaging. One rule: `head[i] = max(own_local_hydrostatic[i], max over connected wet neighbours head[j])`. Averaging is diffusion (O(N^2) settling); a max is Bellman-Ford (O(direction reversals)). Converges in 2 sweeps on every spec scenario including the U-tube; 1 sweep converges nothing (the negative control). `HEAD_FIELD_SWEEPS_PER_TICK = 32` is a safety CAP on an early-exiting loop, not a budget.
- No +/-1 on vertical neighbours: head carries elevation, so "rises going down" and "falls going up" are the same statement. No omega: extrapolating past a max yields a value no neighbour holds.
- COLD SEED every tick; the previous tick's values are never an input. max is monotone, so reading history would ratchet upward and never fall. The field is a pure function of mask + heightmap + material — no hysteresis, no dependence on material flow.
- The field is TOTAL: dry cells hold `head = z` (so `p = 0`). Leaving them stale at 0.0 made every free-fall edge read a large NEGATIVE driving head and sleep — a completely frozen sim.
- Driving-head sites multiply by `GRAVITY_HEAD_SCALE` as well as dividing by `depth_scale`. Omitting it drove liquid edges 25x too weakly, also a frozen sim.
- NO EXPOSED-TOP PIN. Only the free-fall (unsupported) pin remains. The exposed-top pin was redundant (own_elev is already the self-term of the max, and for a column's topmost cell it IS the free-surface elevation) and actively wrong in transit: for a full cell `own_elev` equals EXACTLY the `z` of the air cell above, so the driving head across every water/air interface was identically zero. No surface could rise and no siphon could climb.
- All 7 static specs pass at w=64/128/256/512. Both scoreboards green, dynamic one no longer `#[ignore]`d. Heat-map brightness 165.08/255 vs column_depth's 162.36 (was 47.3 vs 199.9).

## The refuted multigrid pass is DELETED (local commit acf2da48, not yet pushed)

`elliptic_liquid_level_pass` ("Fast liquid levelling (slow)") and everything that existed only to serve it are gone: the `elliptic_head_gate` thread-local, `recompute_column_depth_scoped`, the union-find helpers, seven ELLIPTIC_*/MULTIGRID_* constants, the `elliptic_liquid_level` parameter/field through settle_tick + DrawingSimulation + TestSim + the wasm binding, the UI checkbox and its help paragraph, and six tests/diagnostics. ~1570 lines out of physics.rs. Suite green afterwards (101 passed, 1 intentional marker failure, 43 ignored), all four integration toggle tests pass, warning set unchanged.

`LIQUID_ELLIPTIC_THRESHOLD` was KEPT — despite the name it is the head field's own liquid-only edge gate, not part of the deleted pass.

## Open, and what gates default-on

#64 is the blocker. It now has a second, far cheaper reproducer than the valley diagnostic: `test_liquid_flowing_liquid_does_not_stand_in_walls` (a SHIPPING test, w=64, 0.15s) goes from voids@160 = 6 to 157 with `head_field_transport` forced on. That is a 26x regression on a shipping bound at the LOW resolution — so the transport problem is not confined to w=512 or to the valley geometry. See #64 for the full matrix and the first two steps.

Also open: #62 (warm-start with decay, only if a switchback scenario measures above 2 sweeps), #63 (pressure-sensitive flow rate — note saturation is upstream of it and it cannot fix a full vessel), #65 (unconfirmed heat-map overlay perturbation).

## Standing constraints

Do not modify `pressure_project`, `clamp_edge_feasible`, `support_fraction`, `fresh_overburden_must_blocks`, `recompute_column_depth`. `test_water_blob_stays_left_right_symmetric_under_gravity` FAILS INTENTIONALLY and must never be fixed, weakened, ignored or retitled. Never weaken a test or tune a constant to land inside a passing window. Host has no linker — everything runs inside `distrobox enter sandart-dev`. `cargo check -p sandart-wasm` typechecks nothing without `--target wasm32-unknown-unknown --release`. Integration tests run separately: perfect_simulation_determinism, fresh_pressure_field_toggle, pressure_heatmap_head_field_toggle, head_field_transport_toggle.

Superseded: artifacts/design/TASK55-BRIEF.md describes the averaging design and the multigrid plan, both abandoned. This description is authoritative.
