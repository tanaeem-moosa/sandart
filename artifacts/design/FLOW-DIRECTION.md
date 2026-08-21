# Lateral vs downward: where coarse and fine actually disagree — 2026-08-20

The question: *"in an hourglass simulation comparing coarse with fine, which direction do we have
more flow disagreement? lateral or down?"* — and the clarification that settled the method: *"I am
talking about actual flow of material between blocks."*

## The answer

**Proportionally, the disagreement is lateral. The coarse level moves material sideways at roughly
twice the relative rate the fine level does.**

Grid 512, hourglass, 200 ticks, overclocking on. Realised mass transfer, counted where the transfer
happens (`flux_edge_apply`, past its `MIN_FLUX` cutoff) — mass that actually moved, not a
candidate, a velocity or a reconstruction.

| | lateral | down | lateral/down |
|---|---|---|---|
| **DrySand** fine, all edges | 203.95 | 3893.93 | 0.052 |
| **DrySand** fine, across block boundaries | 27.07 | 486.61 | **0.056** |
| **DrySand** coarse (its every edge is a tile boundary) | 12.38 | 102.68 | **0.121** |
| **Water** fine, all edges | 179.95 | 2568.73 | 0.070 |
| **Water** fine, across block boundaries | 25.30 | 321.56 | **0.079** |
| **Water** coarse | 8.95 | 75.66 | **0.118** |

DrySand: 0.121 vs 0.056, a factor of **2.2**. Water: 0.118 vs 0.079, a factor of **1.5**. Both
levels move far more material down than sideways — gravity is the dominant term in an hourglass
either way — but the coarse level's mixture is consistently and substantially more lateral.

At the shipped geometry a block IS a coarse tile, so "fine, across block boundaries" and "coarse,
every edge" describe the SAME physical boundaries, which is what makes the two rows comparable.

## Why the ratio, and not the absolute numbers

The coarse level holds a tile's height as an AVERAGE rather than a sum (`NestedSim`), so one unit
of coarse flux is `t*t` = 64 units of fine mass; and one coarse cell of displacement is 8 fine
cells of distance. Scaled into fine mass units the coarse level moves ~13x more material across
those boundaries in both directions — which is not a finding, it is the restatement that a coarse
sim transports faster because its cells are bigger (SESSION-HANDOVER §1). **The lateral/down ratio
within a level is scale-free, and it is the only comparison here that carries meaning.**

## The second instrument, and its correction

`diag_delta_direction` asks a different question — not what each level DID, but what transport
would reconcile them: the minimum-energy flux `F` with `div F = delta`, i.e. solve
`lap(phi) = delta` with Neumann boundaries and take `F = grad(phi)`. For DrySand that reconciling
transport is **lateral/down = 0.327**, about 6x more lateral than the fine level's realised 0.056.

**It was wrong the first time.** Plain Gauss-Seidel at 800 sweeps reported 0.534; raising the sweep
count moved it to 0.345 and doubled the magnitudes — the solver had not converged, and the
"answer" was an artifact of the iteration count. Replaced with SOR (omega = 1.9) plus an explicit
residual test: it now runs ~1,220 sweeps per tick to a worst residual of 9.98e-5 and prints both,
so an unconverged run announces itself. The value above is the converged one.

## What follows

Both instruments point the same way: the coarse level wants proportionally more sideways transport
than the fine level performs, and the transport that would reconcile the two states is more lateral
still. That is consistent with the standing explanation for the visible symptom — a pile above its
angle of repose is a fine-scale instability whose correction is lateral, the coarse level's
comparatively lateral flow is exactly the signal that would fix it, and `|Delta|` throws the
DIRECTION away before the scheduler ever sees it.

Two follow-ups this makes concrete, neither built:

1. **A directional signal.** `delta` is a scalar per tile. The reconciling flux `F` above is a
   vector field over the same tiles, and its lateral component is a per-block number that says "the
   coarse level wants sideways movement here". That is a candidate clock signal with an actual
   direction in it, and it is computable — though at ~1,220 SOR sweeps per tick it needs a much
   cheaper solve (a few multigrid V-cycles) before it could run live.
2. **Selective coupling.** Coupling costs ~36% frame time for ~7% spread when applied everywhere.
   The measurement says the lateral disagreement concentrates where the pile is; coupling only
   where the local disagreement is predominantly lateral would buy the part that matters at a
   fraction of the cost.
