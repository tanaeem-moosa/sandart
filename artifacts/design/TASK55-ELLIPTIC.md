# Task #55, second half: elliptic propagation solve

## Bottom line, up front

**The solve does NOT remove the propagation limit.** `ticks_to_halve` still scales with grid
width `w` under `elliptic_head_gate=ON`, at almost the same slope as the shipped one-cell-per-tick
solver. It buys a real but modest, roughly constant *fractional* speedup (~10-18% fewer ticks at
w=128 and w=512 alike) — not the qualitative "whole body moves in one tick regardless of size" fix
the task asked for. At small scale (my own hand-built test geometry, well under production width)
the same mechanism buys a much larger win (up to ~70% fewer ticks for a two-well pocket scenario).
The reason is structural and explained below: this pass runs a *fixed* iteration budget
(`ELLIPTIC_ITERATIONS = 48`) per tick, so its reach per tick is bounded in absolute cell count, not
in domain fraction — exactly the "propagates N cells, not the whole body" problem, just with N≈48
instead of N=1. It is a strictly better constant, not an asymptotic fix.

Given the parallel finding that the free-surface potential half (`multiplicative_lateral_gate`) is
independently refuted at production scale (slower on arch and pockets, bit-identical on the lake,
destroys repose), and that Torricelli/vertical drain rate turned out to dominate the shipped
hourglass anyway — this solve was the remaining candidate for actually fixing #55, and it does not
close the gap at production width. It is a real, measured, mass-conserving, non-overshooting
improvement; it is not the fix.

## What was built

`elliptic_liquid_level_pass` (new function, `sandart-sim/src/physics.rs`), gated by
`elliptic_head_gate` (new module, same `#[cfg(test)]` / `#[cfg(not(test))]` pattern as
`multiplicative_lateral_gate` — default OFF, non-test builds hardcode `false`, no thread-local
read in production). Called once per tick from `settle_tick`, before the phase-0/phase-1 loop,
reading the tick's frozen `heightmap.data` for the driving field and mutating `temp_heights`
directly (a real transfer, not a head adjustment) so the ordinary solver that runs afterward sees
an already-partially-levelled starting state.

**Domain — (a) liquid-only, not (b) an explicit yield criterion.** The task asked me to justify
this choice. I chose (a): the solve's node set is restricted to interior, in-mask cells with
`liquidity(wetness) >= LIQUID_ELLIPTIC_THRESHOLD (0.999)` and `fill > ELLIPTIC_WET_EPS`. I
considered (b) — folding `tau` (the existing per-edge yield stress) into the elliptic problem as an
obstacle/variational-inequality constraint — and rejected it as out of scope for this task's
remaining budget: `tau` is written for a single edge's driving head, not a 2-D Poisson right-hand
side, and retrofitting a proper obstacle problem (`|grad(eta)| <= tau`-style constraint solved
jointly with the Laplacian) is materially more work than a domain restriction. The domain
restriction is empirically justified, not just assumed: `mult_lateral_conveyance` (which I reuse
for edge conveyance) has *no* yield criterion, so if this solve ran over granular material it would
flatten every settled pile in a handful of ticks — confirmed directly, see the repose section
below.

**The math.** `eta[i] = h_ref[i] + column_depth[i] - row(i) * depth_scale`, where `h_ref` is a
cell's own fill lifted into `column_depth`'s reference-row units (reusing the existing
`multiplicative_lateral_gate` call site's own unit-fix machinery). Subtracting `row(i) *
depth_scale` is what makes this comparable ACROSS rows, not just within one row like the existing
multiplicative head's `eta` — proven directly in `test_elliptic_eta_is_row_independent`, and it is
what lets the solve connect two separate wells through a submerged basin at a different row than
either well. Conveyance reuses `mult_lateral_conveyance` at `k = liq = 1.0` (every node is already
near-pure liquid). The solve is Gauss-Seidel (not Jacobi), applied directly to `temp_heights` with
every single edge step individually `clamp_edge_feasible`-clamped against the live buffer —
sequential application is what buys correctness cheaply here in place of `pressure_project`'s
fuller Jacobi + node-degree + arbitration machinery, and is sound (not the same risk as the liquid
WAVE solver's rejection of Gauss-Seidel) because this pass carries no momentum term — it is a pure
relaxation toward a diffusion equilibrium. Sweep direction alternates forward/backward each
iteration so information does not only propagate one way per call.

**Two real bugs found and fixed during this session, worth recording:**

1. **Overshoot / non-convergence.** The first version used `conv_e` (`mult_lateral_conveyance`'s
   raw, unbounded output) directly as the per-edge transfer coefficient. `test_elliptic_residual_
   falls_monotonically` caught this immediately: residual went 0.24 -> 10.9 -> stuck at 12.8
   forever (non-monotonic, in fact strictly worse and then frozen) on a simple smooth-ramp test
   case. Root cause: `mult_lateral_conveyance` was designed to feed `flux_edge_candidate`'s own
   separate momentum/damping pipeline, not to be used as a bare multiplier on a direct mass
   transfer — its output is unbounded (`~h_ref^1.5`), not a dimensionless rate. Fixed by
   introducing `ELLIPTIC_CONV_REF` and a saturating `weight_e = conv_e / (conv_e +
   ELLIPTIC_CONV_REF)` so the per-edge step can never exceed a damped step toward equalising that
   one edge, regardless of how large `conv_e` gets. After the fix, the residual trace is strictly
   monotonically non-increasing (see below).
2. **Adaptive-scheduler discard.** The pass mutates `temp_heights`, but the tick-end "copy back
   updated blocks" step only writes `heightmap.data` for blocks already marked `modified` — a
   block the adaptive scheduler did not otherwise touch this tick stays un-`modified`, so this
   pass's own correction was silently reverted at the start of the very next tick
   (`temp_heights.copy_from_slice(&heightmap.data)`). Measured directly: before the fix, my own
   POCKETS diagnostic showed `ticks_to_halve=None` (never converged in 300 ticks) under the
   adaptive scheduler while converging in 12 ticks under `perfect_sim_tick` on the identical
   scenario — a dead giveaway. Fixed by having the call site walk the pass's own `net_activity`
   output and call `activate_neighbor` (the same mechanism the ordinary flux solver already uses)
   on every touched cell's block and its 4 neighbours. After the fix, adaptive and perfect_sim
   track each other closely everywhere I measured (see the tables below) — a second, independent
   confirmation that what's left is genuine physics, not a scheduling artifact.

## Does it remove the propagation limit? (the key measurement)

Reused the SAME isotropic resolution-scaling construction the parallel resolution-sweep work on
`diag_task55_arch_collapse_rate` uses (`s = w/64`, every coordinate derived from that one factor),
in a new diagnostic (`diag_task55_elliptic_resolution_scaling`) rather than editing that function.
Measured at w=128 (both adaptive and `perfect_sim_tick`) and w=512 (adaptive only — a second
`perfect_sim_tick` pass at w=512 was not affordable in this session's remaining budget; flagged
rather than silently only reporting the cheap width).

| w | mode | gate | ticks_to_halve (raw) | normalised (/ w/64) |
|---|------|------|----------------------:|---------------------:|
| 128 | adaptive | OFF | 128 | 64.0 |
| 128 | adaptive | ON | 115 | 57.5 |
| 128 | perfect_sim | OFF | 136 | 68.0 |
| 128 | perfect_sim | ON | 112 | 56.0 |
| 512 | adaptive | OFF | 522 | 65.25 |
| 512 | adaptive | ON | 466 | 58.25 |

**Reading this plainly:** the OFF row at w=512 (522 raw / 65.25 normalised) matches the parallel
agent's own independent measurement, which is a good cross-check. With the gate ON, the
normalised value drops from ~64-68 to ~56-58 at BOTH widths — a real, reproducible ~10-18%
reduction, but the normalised number does **not** collapse toward a small constant as `w` grows,
which is what "propagates in one tick" would actually look like. It is still clearly scaling with
`w` (raw ticks_to_halve roughly quadruples from w=128 to w=512, same as OFF).

**Why, mechanically:** `ELLIPTIC_ITERATIONS = 48` is a *fixed* per-tick budget, independent of `w`.
A Gauss-Seidel sweep's reach per iteration is on the order of one cell (attenuated further by the
`weight_e` under-relaxation introduced to fix the overshoot bug above), so this pass extends the
shipped solver's one-cell-per-tick reach to something like a few-dozen-cells-per-tick reach — a
better constant, not a fix that scales with the domain. At w=512 the arch's own gap is roughly
320 cells wide; 48 (damped) Gauss-Seidel iterations cannot come close to spanning that in one tick,
so the asymptotic behaviour reverts to being budget-limited, just with a bigger budget than 1.
Raising `ELLIPTIC_ITERATIONS` would likely help further (at direct extra per-tick cost — see
Cost); reaching genuine width-independence would need either a much larger iteration count that
itself scales with `w` (defeating the purpose) or a real multigrid/direct solve, which is beyond
what this task's remaining time allowed to build.

## Does it meet the acceptance criterion? ("arch collapses fast while draining")

**At production-relevant scale (w=512): no, not really — a modest ~11% reduction in ticks-to-halve
is not "collapses fast."** The mechanism (fixed iteration budget, domain-independent reach) is the
same reason as above.

**At small scale it looks much better, and that gap is itself informative.** My own hand-built
diagnostic (`diag_task55_elliptic_propagation`, private geometry — a 64x100 arch-over-void and a
50x50 two-well-via-basin scenario, built separately from the parallel agent's resolution-sweep
work rather than editing it) showed:

| scenario | gate combination | mode | ticks_to_halve |
|---|---|---|---:|
| ARCH (64x100) | mult=OFF, elliptic=OFF | adaptive / perfect | 71 / 65 |
| ARCH | mult=OFF, elliptic=ON | adaptive / perfect | 44 / 47 |
| ARCH | mult=ON, elliptic=ON | adaptive / perfect | 40 / 38 |
| POCKETS (50x50) | mult=OFF, elliptic=OFF | adaptive / perfect | 45 / 45 |
| POCKETS | mult=OFF, elliptic=ON | adaptive / perfect | 14 / 14 |
| POCKETS | mult=ON, elliptic=ON | adaptive / perfect | 14 / 14 |

At this (much smaller) scale, elliptic=ON gives real, large wins: ~35% faster arch collapse,
~70% faster pocket equalisation, and adaptive/perfect track each other almost exactly (confirming
physics, not scheduling, once the block-wake bug above was fixed). The `mult=ON` rows show a
further improvement ON TOP of elliptic in these particular small scenarios — but I want to be
careful not to overclaim this: these are small hand-built grids, not the parallel agent's
production-width resolution sweep, and the coordinator's own w=512 finding is that
`multiplicative_lateral_gate` alone is refuted at production scale (slower on arch and pockets,
destroys repose). I have not independently re-verified the mult+elliptic composition at w=512; I
only measured it at this smaller scale, and I'm reporting it as such rather than extrapolating it
to production width.

**The honest synthesis:** this solve's fixed-iteration reach comfortably covers small domains
(where it looks like a real fix) but is swamped by production-scale domains (where it's a modest
speedup). Since the acceptance criterion is specifically about the shipped, production-scale
behaviour, the answer there is **no, this does not meet it as built.**

## Convergence evidence

`test_elliptic_residual_falls_monotonically` (new, non-ignored, runs in the normal suite) builds a
single row of liquid with a linear fill gradient (every cell genuinely below its own capacity, so
free capacity exists at every edge — see the test's own comment for why a fully-saturated test
scenario, tried first, is a degenerate case that cannot move anything at all, which is correct
behaviour, not a bug) and asserts the function's own returned per-iteration residual
(max `|eta_a - eta_b|` over every live edge) is non-increasing. After the overshoot fix above, the
measured trace over 48 iterations is strictly monotonically non-increasing:

```
[0.242165, 0.242153, 0.242142, 0.242126, 0.242119, 0.242104, 0.242088, 0.242069, 0.242054,
 0.242035, 0.242020, 0.241997, 0.241978, 0.241955, 0.241932, 0.241905, 0.241882, 0.241856,
 0.241829, 0.241798, 0.241772, 0.241737, 0.241711, 0.241673, 0.241642, 0.241600, 0.241566,
 0.241531, 0.241497, 0.241451, 0.241417, 0.241371, 0.241325, 0.241283, 0.241238, 0.241192,
 0.241146, 0.241096, 0.241047, 0.240986, 0.240929, 0.240871, 0.240814, 0.240757, 0.240696,
 0.240635, 0.240570, 0.240498, 0.240429]
```

Note the decay is SLOW in absolute per-iteration terms for this metric (max adjacent-cell
difference) — worth being honest about, since it's the same underlying phenomenon as the
resolution-scaling result above: `weight_e`'s saturation and `ELLIPTIC_EDGE_OMEGA=0.5`'s damping
were chosen for stability (never overshoot), which trades away convergence speed. The row's
overall spread (max-min across the whole row, a coarser but more representative measure of "has it
flattened") shrinks by more than 10% over the same 48 iterations (asserted in the same test);
mass is conserved to within 1e-3 (asserted). I did not sweep `ELLIPTIC_ITERATIONS` or
`ELLIPTIC_EDGE_OMEGA` against a target — per this task's own instruction not to tune constants to
land inside a passing window, both are reported as un-tuned starting choices with the reasoning
in their own doc comments, and their cost/benefit tradeoff is visible directly in the
resolution-scaling numbers above rather than hidden behind a single pass/fail bound.

## `test_dry_sand_has_angle_of_repose` with `elliptic_head_gate` forced ON

**PASS.** Ran the existing (unmodified) test directly under `elliptic_head_gate::set_enabled(true)`
via `std::panic::catch_unwind` in my own diagnostic. Actual measured values (unaffected by the
gate, as expected, since the domain restriction means the granular path never enters this solve at
all):

- CASE 1 (steep): initial=0.3500 (19.29 deg) -> final=0.0886 (5.07 deg), total_flow=412.70
- CASE 2 (shallow): initial=0.0532 -> final=0.0534 (3.06 deg)
- CASE 3 (at angle): initial=0.0886 -> final=0.0760 (4.34 deg)
- CASE 4 (deposit on peak): flank_slope=0.0972 (5.55 deg), total_flow=74.00
- NON-VACUITY ANCHOR @450 ticks: DrySand=0.0652 (3.73 deg), Water=-0.0000 (-0.00 deg)

DrySand keeps a real, nonzero repose angle; Water reads flat (as it always does — Water has no
yield stress with or without this gate). This is the expected, unsurprising result given the
liquid-only domain restriction, and it's the concrete evidence that restriction actually holds in
this build, not just in the doc-comment reasoning.

## Cost

**Not measured to completion.** `cargo run --release --example bench_sandfall -- --ticks 600
--materials water,drysand` with the gate forced ON (via a temporary edit to the
`#[cfg(not(test))]` twin, reverted before finishing — the gate cannot otherwise be reached from a
non-test binary, since `elliptic_head_gate::set_enabled` is `#[cfg(test)]`-only, by design, same as
every other gate in this file) ran for over 12 minutes of wall clock at 96% CPU in the background
without finishing, against an expectation (based on this codebase's usual per-tick costs) of well
under a minute for a normal 600-tick run. I stopped waiting on it per instruction rather than
continue blocking.

That itself is informative and I am reporting it rather than guessing a number: this pass, run
unconditionally over the whole 512x512 grid every tick with a (up to) 48-iteration inner sweep, is
expensive — plausibly by a large factor (likely 10x+, extrapolating from how far past a
sub-one-minute expectation it ran without completing), not a rounding error. I did not get a
precise ms/tick number and am not going to fabricate one. `bench_sandfall`'s materials list is
`water,drysand`; only the WATER portion should pay this pass's real cost (DrySand cells never enter
the liquid-only domain, so the DrySand portion should cost close to the gate-off baseline plus a
cheap O(grid) domain-membership scan) — I was not able to confirm this split empirically either,
for the same reason.

**What this means for the bottom line:** even the modest ~10-18% ticks-to-halve improvement
measured above comes at a per-tick cost that appears to be at least an order of magnitude, for a
pass that runs unconditionally over the full grid every tick regardless of whether any liquid body
is actually out of equilibrium. Cheapening this (e.g. only running when some cheap signal suggests
a genuine disequilibrium exists, or restricting to touched/active blocks the way `pressure_project`
does rather than a full-grid domain scan every tick) would be necessary before this could be a
serious candidate for shipping, independent of the scaling problem above.

## Overall verdict

Built, working, mass-conserving, monotonically convergent (in the narrow sense measured), and
correctly restricted to liquid so it does not damage granular repose. It does NOT solve task #55:
the propagation limit is reduced by a constant factor (bounded by a fixed iteration budget), not
removed, so it degrades back toward the original one-cell-per-tick scaling at production width —
and its per-tick cost, uninvestigated further given the time this session had left, appears large
enough that the modest win it does provide may not be worth shipping as built. Between this and the
now-refuted multiplicative potential half, task #55's acceptance criterion ("arch fixes itself even
with outflow, not arch can't happen") is still open. What I'd try next, with more time: (1) make
the per-tick cost proportional to actual disequilibrium rather than a full-grid scan every tick,
(2) either scale `ELLIPTIC_ITERATIONS` with the connected component's own diameter (at the cost of
reintroducing an `O(w)`-ish per-tick cost, but now concentrated only where needed) or replace the
fixed-iteration Gauss-Seidel with a real multigrid V-cycle, which is the standard way to get
`O(log N)` convergence for exactly this kind of elliptic problem instead of `O(N)`.
