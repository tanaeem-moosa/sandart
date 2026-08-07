# #57 — 2.34 — BLOCKS making the fresh pressure field default: arches do not COLLAPSE fast enough. It makes an existing failure worse, not a new one

**Status:** pending

---

## THIS TICKET NOW BLOCKS A REQUESTED CHANGE (2026-08-07)

USER ASKED: make the fresh pressure field the new behaviour and delete the toggle. NOT DONE — it fails a shipping test today, and the only way to land it right now would be to weaken that test, which is forbidden.

Re-measured 2026-08-07 by flipping the defaults and running the suite. `test_liquid_flowing_liquid_does_not_stand_in_walls` (w=64), `voids@120 / voids@160`, bounds <=150 and <=20:

    both off (shipped default)      :  60 /   6   PASS
    fresh_pressure_field on         : 142 /  66   FAIL  (voids@160 = 66, bound is 20)
    head_field_transport on         : 239 / 157   FAIL
    both on                         : 239 / 157   FAIL

66 is the SAME number this ticket recorded in 2026-08-05. Nothing shipped since then moved it — in particular the max-propagation head field (#55) did not, because with `head_field_transport` off the head field never touches a liquid edge, and with it ON the walls test gets far worse still (157). So the #55-fixes-this hypothesis below is now UNSUPPORTED by the only metric available; #55's transport path currently makes this metric worse, not better (tracked in #64).

Note also that "both on" is bit-identical to "transport on": once head-field transport is enabled, `fresh_pressure_field` is a no-op for liquid, because the head-field branch replaces the `column_depth`-derived driving head on every liquid edge. `column_depth` freshness only still matters for granular and mixed edges. If head-field transport ever becomes the default for liquid, THIS TICKET SHRINKS to a granular-only question.

TO UNBLOCK, one of: (a) find and fix what makes arches persist under the fresh field, so voids@160 comes back under 20; or (b) a deliberate user decision to accept the regression, with the walls test's bound changed as an explicit, documented product decision rather than as a way to make a change pass. (b) is the user's call, not the implementer's.

---

USER CORRECTION 2026-08-05, after visually confirming the arch on the deployed build: "it is makes existing issue worse, doesn't introduce new issue. arching without this. and it doesn't fix arching fast enough right now."

This reframes the whole ticket. The fresh pressure field does NOT create arching. Arching happens anyway; the fresh field makes it worse and slower to resolve. So the question is not "what did the standalone pass break" but **"what dissolves an arch, and why is it too slow"**.

The measurements fit that reading exactly:
- baseline (inline field): 6 voids, 4 of 5 runs single-cell, longest 2, largest 2D region 3 cells, **max persistence 5 of 20 sampled ticks** — arches form and COLLAPSE.
- fresh field: ONE contiguous 55-cell region, runs up to 18, 29 cells present in >= 15 of 20 ticks, **several at 20/20** — arches form and DO NOT collapse.

Same phenomenon, different collapse rate. Not a new defect.

## Why this was expected to unify with the 45-degree cone — NOW IN DOUBT

An arch collapses when the pressure imbalance beneath it is felt by the material holding it up. In an explicit scheme that information travels ONE CELL PER TICK, so a wide arch takes many ticks to learn it is unsupported — long enough to restabilise. That is the same propagation limit that produces the 45-degree characteristic cone (#45) and prevents connected pockets equalising (#55).

USER DECISION 2026-08-05: committed to pressure with fresh fields; fix the issues rather than back out. Sequence agreed: decouple the fresh field for scheduling first (unblocks #47), then the elliptic head solve.

CAVEAT ADDED 2026-08-07: the elliptic head solve has now shipped (as the max-propagation head field, #55) and does NOT rescue this metric. Either the propagation-limit theory is wrong for arches, or the head field's liquid-only scope means it never reaches the granular material actually forming the arch. The second is the more likely reading and is worth checking before more work goes into the first — the arch scenario is granular, and `LIQUID_ELLIPTIC_THRESHOLD` gates the head field off entirely for granular edges.

## STATUS: the fresh field is SHIPPED behind a toggle (ad7fdad)

"Fresh pressure field (experimental)", third in the Debug group, default OFF. Off is bit-identical over 200 ticks; on diverges (both asserted in `sandart-sim/tests/fresh_pressure_field_toggle.rs`, so it cannot pass vacuously). Forced on inside the walls test it reproduces `voids@120=142 voids@160=66 total=20082` exactly.

USER HAS VISUALLY CONFIRMED THE ARCH IS REAL on the deployment. The test is not misleading us.

## RULED OUT — all measured, do not re-run

1. **Voids inherit overburden and repel material.** `column_depth` in the empty gap cells measures **0.0000**.
2. **Single-cell gaps break the `resting_above` chain.** The chain does not reset at a gap; `resting_above` clamps to 0 for that row but `column_depth[above]` carries through.
3. **Buffer-plumbing bug making the fresh field read ~zero.** At tick 0 it is BIT-IDENTICAL to inline (24.0 / 64.0 / 104.0 / 144.0 down a resting column).
4. **Missing candidate edge into the void.** Within the void's rows the head on both sides is ~0; where real head exists (row 31: 20.9 vs 0.5) flux DOES occur (x=26 goes 0 -> 0.84 in that tick).
5. **Coefficient mis-tuning.** 20-point sweep of `LATERAL_PRESSURE_SCALE` over [0, 20] never gets voids below 37.

Also tried and WORSE, unexplained: running the pass once per PHASE over `temp_heights` instead of once per tick over `heightmap.data` gives 85 and breaks the tick-120 bound too.

## A SMALLER SEPARATE FINDING, and a correction to how it was framed

`in_transit_at` under-reports magnitude for fast drainage: at x=40, pre-tick `h` = 0.77-1.00 across rows 28-35 draining to 0-0.19 by the end of that same tick, but `in_transit_at` reported only 0.0-0.55. Not staleness (`block_will_run = true`, `ticks_since = 1`, `edge_vel_v` a normal 0.02-0.63).

USER CORRECTION: "is pressure really 0 if there is sustained flow in a cell?" It is not. Supported material transmits load whether or not it is moving — the basis of Janssen/Beverloo. The zero-pressure case is FREE FALL (ballistic, no contact force), not flow. So `in_transit_at` keying on edge velocity is a reasonable proxy for supported-vs-unsupported and is directionally right.

An earlier version of this ticket called it a defect and made fixing it a PREREQUISITE for the standalone pass. Both wrong. The claim shrinks to an under-report of magnitude for fast drainage. Worth fixing on its own merits; NOT established as connected to the arch; NOT a prerequisite for anything.

## Context that stands regardless

The inline computation leaves most of the grid stale — tick 39, 467/1785 interior cells untouched; tick 118, 821/1785; **tick 164, 1126/1785 (63%), entire top-right quadrant 100% stale**; max |stale - fresh| 39.6 against typical magnitudes of 24-144. NOT budget throttling: a block with zero displacement and staleness < `MAX_STALENESS` (30) never enters any candidate list, so `budget_n = usize::MAX` does not make it run. `LATERAL_PRESSURE_SCALE = 5.0` was tuned against that field.

## Housekeeping

The "voids@160 was already at 19 on plain Jacobi" note at physics.rs ~2930 is STALE — `git blame` traces it to 11cb1775 (2026-08-01), before ~10 intervening physics commits. Current baseline is 6.

The two `diag_task55_composition_*_w512` diagnostics that crossed `fresh_pressure_field` against `elliptic_head_gate` were DELETED on 2026-08-07 along with the elliptic pass itself (see #55). If a composition question comes back, the surviving axis to cross against is `head_field_transport`.

## Cross-links

#55 (the head field — shipped, and does NOT fix this), #64 (head-field transport regresses this same test 26x), #47 (slabs; the fresh field is wanted there for SCHEDULING only, which is unaffected by the arch), #54 (the standalone pass), #56 (damping vs real symmetry fix), #45 (the 45-degree cone — same propagation limit).
