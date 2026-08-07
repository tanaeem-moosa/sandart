# #37 — 2.14 — wave_params relaxation rate is per-tick, so it scales with resolution in TIME

**Status:** pending

---

Diagnosed 2026-07-30 while chasing the residual resolution dependence left over after #35. Same bug CLASS as #35 but in the TIME dimension rather than space, which is why no amount of spatial analysis found it.

## The mechanism
`wave_params`'s `(c_sq, damping)` — Water is `0.24 / 0.98` — set a per-tick relaxation rate expressed in GRID-CELL units, and they are reused by the cross-gravity lateral edge. Free fall is CFL-pinned at ~1 cell/tick regardless of resolution, so a fixed PHYSICAL disturbance takes proportionally more ticks to level as the grid refines, while the per-tick rate stays constant. A damping constant tuned per-tick at one resolution is effectively a different physical relaxation rate at another.

## Evidence
- The void cells sit where `column_depth` is SMALL (0.07-0.34 against a domain max of ~23-24). They are NOT the hydrostatic-overburden population `LATERAL_PRESSURE_SCALE` addresses at all — they are ordinary free-surface relaxation lag in the upper chamber during active drainage.
- The defect is TRANSIENT at every scale (always reaches zero, verified out to 20x normal ticks), but time-to-zero grows FASTER than the harness's linear CFL scaling: ~100-110 ticks at s=1, ~260-300 at s=2, ~1150-1200 at s=4. Effective exponent rises ~1.4 -> ~2.1, consistent with diffusive/parabolic relaxation superposed on a fixed local time constant, not a purely advective process.
- Normalising the metric by BOTH area and tick count still shows real growth: 2.5% -> 5.6% -> 13.0% -> 26.6% across s=1..8. So part of the raw growth is mechanical metric inflation, but a substantial genuine defect remains underneath.
- **Decisive: it is not tunable.** Sweeping `LATERAL_PRESSURE_SCALE` at 512 from 5 to 10/20/40/60/100 drops the walls total to a hard floor of ~66.7-66.8M by 40 and never improves further — roughly 2000x over the 34,000 bound — while stream coherence stays comfortably inside its bound the whole way (9.57% of width at 100). No value of that constant fixes this.
- Consistency check that validates #35's arithmetic: at 512, `LATERAL_PRESSURE_SCALE = 40` with #35's fix reproduces 66,730,129 almost exactly — the historical pre-fix number — confirming that post-fix 5.0 at reference 512 is the same physical driving strength as pre-fix 5.0 was.

## Ruled out while finding this
- `MIN_FLUX`, `FLOW_INACTIVE_THRESHOLD`, `MUST_SIMULATE_THRESHOLD`, `in_transit_at` — all intensive per-cell fill fractions (~0-1.5 regardless of grid size), not running sums. No resolution dependence.
- Block-LOD: `budget_n = usize::MAX` in these tests forces every block to simulate, and `block_size` is 32 at every scale, so scheduling cannot be inflating the numbers.

## Why this is harder than #35
`wave_params` also drives the g=0 Sandbox wave solver and carries its own documented spectral-radius stability margin, with several tests pinned to current values. This is not a constant to retune casually — changing it safely means understanding the stability analysis first. Treat as architectural, not a tuning pass.

## Practical consequence for the resolution switch (#36)
Once the grid becomes user-selectable, the simulation will STILL look somewhat different at 64 versus 512, because of this. That difference is now diagnosed and attributable — it is NOT #35 resurfacing, and it is NOT introduced by the switch. Expect it and say so before anyone reports it as a regression.

## Also noted, needs confirmation
ms/tick at 512 measured 3.77 in one run under distrobox, against a reference of ~1.37 recorded earlier. Single measurement, different conditions (budget_n=MAX, hourglass-drain scenario, 800 ticks after 200 warmup). Re-measure under controlled conditions before treating it as a perf regression.
