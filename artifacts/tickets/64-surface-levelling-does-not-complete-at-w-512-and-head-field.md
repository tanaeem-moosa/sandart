# #64 — 2.41 — Surface levelling does not complete at w=512, and head-field transport levels SLOWER than legacy there

**Status:** pending

---

MEASURED 2026-08-07 by `diag_task55_surface_valley_fills` (in `task55_head_spec.rs`, `#[ignore]`d diagnostic). Scenario: a flat-bottomed open vessel filled level, with a deep narrow valley carved out of the middle of its free surface, run 600 ticks.

    w=64   transport=false : valley row 48 -> 29   (surrounding surface 26)  risen 19  flow 3296
    w=64   transport=true  : valley row 48 -> 27   (surrounding surface 26)  risen 21  flow 14785
    w=512  transport=false : valley row 384 -> 286 (surrounding surface 205) risen 98  flow 1058632
    w=512  transport=true  : valley row 384 -> 311 (surrounding surface 205) risen 73  flow 1187180

TWO SEPARATE FINDINGS, do not conflate them:

(1) LEVELLING DOES NOT COMPLETE AT w=512. At w=64 the valley reaches within 1-3 rows of the surrounding surface. At w=512 it climbs 98 of the 179 rows it needs and is still 81 rows short after 600 ticks. Some of this is expected — 600 ticks is proportionally less physical time at higher resolution — but quantify it against a resolution-invariant time base before accepting "just slower". Check #37 (2.14) FIRST: it records that `wave_params` relaxation is per-tick and therefore scales with resolution in TIME, and this may simply be that.

(2) HEAD-FIELD TRANSPORT IS WORSE THAN LEGACY AT w=512 (73 rows vs 98) WHILE BEING BETTER AT w=64 (21 vs 19), AND MOVES MORE MASS DOING IT (flow 1187180 vs 1058632). Mass is moving more and accomplishing less. This is the more important one: it gates whether `head_field_transport` can ever become default-on, which is the actual goal of #55.

## SECOND, CHEAPER REPRODUCER — ADDED 2026-08-07

`test_liquid_flowing_liquid_does_not_stand_in_walls` (physics.rs, a SHIPPING test, w=64, runs in 0.15s) shows the same regression far more sharply than the valley diagnostic, and needs no `--ignored` run. Forced-on matrix, `voids@120 / voids@160` (test bounds are <=150 and <=20):

    both off (shipped default)      :  60 /   6   PASS
    fresh_pressure_field on         : 142 /  66   FAIL
    head_field_transport on         : 239 / 157   FAIL
    both on                         : 239 / 157   FAIL

Two things fall out of that table:

- `head_field_transport` alone takes voids@160 from 6 to 157, a 26x regression on a shipping bound, at w=64 — the resolution where the VALLEY diagnostic said head-field transport was slightly BETTER than legacy (21 vs 19 rows). So the transport regression is NOT purely a high-resolution effect and NOT specific to the valley geometry. Use this test as the primary instrument: it is 400x faster than the valley diagnostic and it is already a bound the project must not break.
- "both on" is bit-identical to "transport on". Once head-field transport is enabled, `fresh_pressure_field` makes no difference to a liquid scenario at all, because the head-field branch replaces the `column_depth`-derived driving head on every liquid edge. `column_depth` freshness only still matters for granular and mixed edges.

LEADING HYPOTHESIS FOR (2) — OVER-DRIVEN, OSCILLATING. The head-field branch has NO bound on its driving head, and the driving head it produces is enormous next to the legacy branch's. `head_scale = GRAVITY_HEAD_SCALE / depth_scale`, so at w=512 (depth_scale = 1) it is 25, and a measured 236-reference-row head difference becomes ~5900 — against a legacy `base_head` of 25 plus a `vertical_bonus` that is itself CFL-capped at `base_head * VERTICAL_PRESSURE_CAP_MULT`. A driving head two orders of magnitude past what the solver was tuned for will saturate the donor/acceptor clamp in `flux_edge_candidate` every tick; if the sign alternates tick to tick, the result is sloshing: maximum flux, near-zero net transport. That matches the measured signature (higher total_flow, lower net rise). NOTE the walls-test numbers above now WEAKEN the resolution half of this hypothesis — `head_scale` is only 3.1 at w=64 and the regression is severe there too — so the unbounded driving head may be the whole story regardless of resolution, or there may be a second mechanism. Do not assume the resolution axis is load-bearing.

RULED OUT, do not chase: `VERTICAL_PRESSURE_CAP_MULT` (#60) is NOT involved. Verified 2026-08-07 — `vertical_bonus` and its cap are computed only inside the LEGACY `else` branch of the driving-head selection. The head-field branch bypasses both entirely. #60 is a legacy-path issue and fixing it would not touch this.

FIRST STEPS, in order: (a) instrument the sign of the per-edge flux over consecutive ticks in `test_liquid_flowing_liquid_does_not_stand_in_walls` with `head_field_transport` forced on, and confirm or refute alternation — that is a direct test of the hypothesis and needs no code change beyond a counter; (b) if confirmed, the fix is a CFL-style bound on the head-field driving head analogous to the one the legacy branch already has, NOT a scaling constant tuned until the test passes.

CONTEXT THAT MATTERS. Under max-propagation, head is UNIFORM through a connected body at rest, so the LATERAL drive between two adjacent surface cells is correctly ZERO. A valley fills VERTICALLY — the air cell above holds `head = z` (low), the body head is high, and that difference lifts water in. Any investigation starting from "why is lateral flow not filling the valley" is starting from the wrong model.

DO NOT start by tuning a rate constant. DO NOT weaken the walls test's bounds; it is the instrument.
