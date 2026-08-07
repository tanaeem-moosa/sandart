# #66 — 2.43 — Advancing the head field costs +219% ms/tick at w=512; it allocates six whole-grid Vecs every tick

**Status:** pending

---

MEASURED 2026-08-07 by `diag_pressure_sensitive_flow_cost_w512` (in `sandart-sim/tests/pressure_sensitive_flow_toggle.rs`, `#[ignore]`d, run with `--ignored --nocapture`). 200 ticks, w=512, Hourglass filled with Water:

    head field NOT advanced   :  6.2 ms/tick
    head field advanced       : 19.9 ms/tick   (+219%)

That is ~160fps down to ~50fps of simulation budget, before rendering. It is a 3.2x slowdown for a pass whose own doc comment describes it as "2 sweeps, O(wet_cells), fixed regardless of resolution".

ISOLATED, so this is not a guess about which toggle causes it. The diagnostic has three arms: off, head-field-advance-only (via `pressure_heatmap_head_field`, which advances the field and changes nothing else), and `pressure_sensitive_flow` on. The middle and last arms measure 19.889 and 19.836 ms/tick — indistinguishable. So the ENTIRE cost is `advance_head_field` itself; the #63 rate arithmetic (two multiplies and a compare per liquid edge) is free by comparison.

THIS IS NOT A #63 REGRESSION. It is the pre-existing cost of the head field, previously paid only by `head_field_transport` and the pressure heat-map's "use new head field" source, and now also by `pressure_sensitive_flow`. Anyone who has turned on the pressure heat-map's new-field source has been paying it. Worth checking whether this is what is actually behind #65's "turning on the heat map overlay somehow messes with the simulation" report — a 3.2x tick-cost jump changes how many blocks the adaptive scheduler can afford per frame, which is a real behavioural change, not an imagined one. (#65 names the BLOCK heat map, a different toggle, so confirm before assuming.)

LEADING SUSPECT, not yet confirmed: `advance_head_field` allocates several whole-grid `Vec`s on every single call — `effective_support_transitive`, `z_elev`, `own_elev`, `pin_target`, `wet_order`, `wet_order_rev`. At w=512 that is six allocations of ~262k elements per tick, plus the zero/copy traffic to fill them. The two relaxation sweeps themselves touch only wet cells and cannot plausibly account for 13.6 ms.

FIRST STEP: confirm the allocation hypothesis before optimising anything — profile or simply hoist the buffers into caller-owned scratch (the same pattern `column_depth`/`head_field` already use: persistent buffers owned by `DrawingSimulation`, resized in `settle_tick`) and re-run the diagnostic. If the number does not move, the sweeps or the transitive-support pass are the cost and the buffers were a red herring.

DO NOT respond to this by reducing `HEAD_FIELD_SWEEPS_PER_TICK`. It is already an early-exiting cap that measures 2 sweeps in practice; the cost is not in the loop count.

Cross-links: #55 (the head field), #63 (third consumer of the advance), #65 (possible explanation), #53 (the same shape of finding for `pressure_project` — fixed per-phase cost, not the iteration loop).
