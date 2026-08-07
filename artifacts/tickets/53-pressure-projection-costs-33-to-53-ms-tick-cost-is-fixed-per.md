# #53 — 2.30 — Pressure projection costs +33% to +53% ms/tick; cost is fixed per-phase, not the Jacobi loop

**Status:** pending

---

Shipped knowingly with the pressure projection in ddd9658 (#45). Not a regression to hunt — a known, measured cost with a diagnosed shape.

## Measured, both runs back to back on the same host (bench_sandfall, 512, Hourglass, 900 ticks)

    material  budget   before -> after   delta
    Water       1024    7.77 -> 11.93    +53%
    Water        256   10.37 -> 14.19    +37%
    Water         32    9.13 -> 12.28    +34%
    DrySand     1024    9.88 -> 13.11    +33%
    DrySand      256    9.83 -> 13.09    +33%
    DrySand       32    8.95 -> 11.87    +33%

Measured by stashing the diff and re-running immediately. An implementation report quoted +45%/+69% against baselines of 8.25/7.83; those were not reproducible — DrySand baseline is 9.88 and its real cost is +33%, not +69%. Use the table above.

## The cost is NOT the Jacobi loop

At ONE iteration DrySand was already most of the way to its 8-iteration cost. So this is fixed per-phase work proportional to touched-edge count, not the sweeps. Cutting `PRESSURE_ITERATIONS` therefore buys almost nothing AND would drop below the convergence plateau (24 and 64 iterations are bit-identical to 8), which would reintroduce the defect `clamp_edge_feasible` fixed. Do not "optimise" this by reducing iterations.

Two easy wins were already taken during implementation and are NOT available again: five passes over the touched lists fused into two, and `cell_capacity_for` hoisted out of the Jacobi loop into a per-node `pressure_cap` cache (wetness cannot change mid-solve). Together worth -17% to -20%.

## Where to look

- A cheaper touched-edge representation. The per-phase setup walks `touched_h`/`touched_v` to build the node set, `fstar`, `degree` and `cap_cache`. That is several passes over the active edge set every phase, twice per tick.
- A minimum-touched-count skip: below some active-edge count the projection cannot be buying anything, so skip the solve and just run the accumulate pass.
- Whether the solve needs to run in BOTH phases, or whether one phase per tick is enough.
- Whether `nodes`/`degree` can be kept incrementally across ticks instead of rebuilt, given the touched set changes slowly.

This is scheduler/bookkeeping work, not solver work. Related to #50, which is the broader "make LOD degrade quality not correctness, then reduce how often it drops" ticket — the two should probably be done together, since both are about the cost and behaviour of the active set.

## Also noted at the same time: Water's mass_rel_err moved

7.88e-10 -> 3.55e-7 over 900 ticks (DrySand untouched at ~1e-10). Conservation is still STRUCTURAL — a correction is one scalar per edge, debited and credited symmetrically, and the clamp clamps that same shared scalar. This is f32 precision loss from larger corrected fluxes accumulating, and it grows with tick count (2.4e-8 at 200 ticks). 0.00004% of mass, so not urgent, but this project treats the 1e-9..1e-8 band as meaningful and Water is now outside it. Worth a look if anyone touches the accumulation, e.g. whether the running sum can be f64.
