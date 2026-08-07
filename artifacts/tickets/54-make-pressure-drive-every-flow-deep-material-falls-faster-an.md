# #54 — 2.31 — Make pressure drive EVERY flow: deep material falls faster, and material next to empty space spreads into it

**Status:** pending

---

USER DIRECTION 2026-08-05: "make pressure impact every flow so that deep water falls faster and empty space next to water spread sideways under pressure. (both for sand at a lower magnitude)"
And on scaling: "lower pressure should have the value lower than now. and higher pressure higher."

## STATUS: HALF SHIPPED in bb0633e

**Delivered:** deep material falls faster. Overburden now feeds the VERTICAL driving head (`VERTICAL_PRESSURE_SCALE`, discounted by `k_of_liquidity` so sand is lower magnitude), shaped by `janssen_effective_depth()` = `z_c*(1-exp(-z/z_c))` — identity for water, saturating for granular — applied at both the lateral and vertical read sites. CFL-capped at `VERTICAL_PRESSURE_CAP_MULT = 1.0` x the existing gravity head.

**NOT delivered, still open:**
1. "empty space next to water spread sideways under pressure" — the LATERAL term is untouched.
2. The redistribution. Shallow material behaves exactly as before; near the surface the Janssen transform is ~identity (slope 1 at z=0), so shallow lateral behaviour is bit-identical. Deep is up, shallow unchanged — NOT "lower pressure lower than now".
3. `JANSSEN_DEPTH_SCALE = 24.0` is unmeasured. The check that would validate it: drain rate vs fill fraction should go FLAT under saturation (Beverloo). Never run.

## THE STANDALONE `column_depth` PASS DID NOT SHIP — do not re-attempt blindly

Moving `column_depth` out of the block loop into an unconditional pass looked like the biggest win available and is NOT in bb0633e. It broke `test_liquid_flowing_liquid_does_not_stand_in_walls`: voids@160 5 -> 66 against a <= 20 bound.

Three attempts, all failed, all measured:

- **Full sweep of `LATERAL_PRESSURE_SCALE`, 20 values in [0, 20].** voids@160 NEVER drops below 37. Not a tuning problem. (Beware how this was first reported: scale=0.0 was called "identical to none of the changes existing", which is WRONG — it also deletes the pre-existing lateral pressure baseline had at 5.0. Read correctly, the sweep shows pressure REDUCES voids: deleting it gives 67, and the new tree with pressure nominally on sat at 66, i.e. as if pressure were doing nothing.)
- **Once per PHASE over `temp_heights`** instead of once per tick over `heightmap.data`, on the theory phase 1 was fed a pre-phase-0 field. WORSE: 85, and it broke the tick-120 bound too.
- **Buffer-plumbing bug making the field read ~zero.** REFUTED by direct measurement: at tick 0 the standalone pass is BIT-IDENTICAL to the old inline computation (24.0 / 64.0 / 104.0 / 144.0 down a resting column). The tick-29 divergence is real trajectory divergence — `h` itself has diverged by then — not a defect.

So the standalone pass computes a CORRECT field and still produces a worse trajectory on that test. Genuine open question. The dead `recompute_column_depth` is kept in physics.rs, unused, with the measurements on it.

## Measurements at bb0633e

- Suite 92 passed / 1 intentional fail / 20 ignored (the extra ignored is a new dam-break diagnostic, `diag_lateral_pressure_term_magnitudes`).
- Blob test: even 3.868e-2 / 1.277e-3 / late_run 43; odd 4.555e-2 / 4.811e-3 / late_run 47. Baseline was 75/75. `final` dropped substantially, `worst` slightly worse.
- `test_liquid_stream_stays_coherent` max_width 9, unchanged from baseline.
- Drain order f_50 @nw=0.02: 0.6331, was 0.6429. SLIGHTLY WORSE, away from the 0.7739 ideal. Recorded, not explained.
- bench_sandfall 512: Water 11.72 -> 12.71 ms/tick (+8%), DrySand 13.14 -> 12.65 (-4%).

## Step 2's measurement, which contradicted the brief's expectation

A dam-break diagnostic (left in the file, ignored) measured `depth_term/tau` for DrySand at **675x at depth 5 rising to 13,050x at depth 60** — tau is a flat 0.08, the depth term is not. And material DOES spread: at depth >= 5 both Water and DrySand go from `h_a=1.0, h_b=0.0` to near-equalisation in a SINGLE tick.

So the yield-stress-cancels-lateral-pressure hypothesis (carried from #52) is REFUTED for this geometry. The lateral term is dominant and material does spread when there is a clean free face. Whatever #52's vertical striping is, it is not sand being unable to move sideways at depth.

NOTE the first probe was a measurement artifact and was discarded: sampling the dead centre of a symmetric fill shows zero driving force BY CONSTRUCTION regardless of pressure. Do not repeat that mistake.

## CROSS-LINKS

- #56 — the damping-vs-real-fix discriminator. Partly answered: under the standalone pass, `worst` collapsed 3 orders while `late_persistent_run` stayed pinned at 75. That is damping. What shipped moves persistence instead (75/75 -> 43/47) — but that metric bounced non-monotonically across the sweep (50/0, 75/75, 74/0), so treat as fragile.
- #52 — its lead hypothesis is now refuted; needs a new one.
- #49 — free-fall acceleration; must be reconciled with the CFL cap here or they stack.
- #33 — the sideways half of this ticket is still that ticket's subject.
- #55 — unaffected; still needs an elliptic head solve.
