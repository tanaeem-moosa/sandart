# Research: single-edge arbitration and n-step advance

**Research only — nothing built, nothing measured.** Written 2026-08-20 against `ba9aff13`, in
response to: *"research, not code, how to correctly do single edge arbitration or n step."*

---

## 1. What arbitration currently is

It is a **Zalesak (1979) flux-corrected-transport limiter**, and it is already the good version of
itself. Per phase:

1. **COLLECT** — each edge computes a *candidate* flux, clamped by its own donor/acceptor limits.
   This is a per-edge upper bound only.
2. **ARBITRATE** — for each cell, sum the raw candidate claims against it. `edge_arbitration_scale`
   returns `min(donor_ratio, acceptor_ratio, 1)`, and multiplying a candidate by it can only shrink
   the claim, never grow or flip it.
3. **APPLY** — move the scaled mass, advect properties.

The reason it exists: multiple edges reading the same frozen donor (or targeting the same acceptor)
can *together* claim more than the donor holds or the acceptor can take, even though each is
individually legal.

**It is single-pass, and provably so.** For donor `d` with `out_scale(d) = min(1, avail(d)/out_total(d))`,
scaling every outgoing edge by at most that factor bounds their sum by exactly `avail(d)`. The bound
is exact rather than approximate, so a second pass recomputing totals from already-scaled fluxes
would find nothing to correct. The min-of-two-ratios construction is precisely what buys that.

**So "do single-edge arbitration correctly" is already answered — there is no iteration to remove.**
The cost is not that arbitration is done badly; it is that it is done *per edge, per step*, and
overclocking multiplies the step count.

That reframes the question into two separate ones, below.

---

## 2. Can arbitration be done once for n steps? No — but it can often be skipped

**Why it cannot be precomputed.** The scale factor is a function of `avail(d)` and `freecap(a)`,
which are the *current* mass and headroom. Both change after every step. It is a nonlinear
projection onto the feasible set (non-negative, under capacity), and the feasible set moves with the
state. There is no fixed scale valid across n steps.

**But it is a provable no-op whenever nothing is contested.** `edge_arbitration_scale` returns
exactly `1.0` when `out_total <= avail` and `in_total <= freecap`. The code already notes that a
cell touched by only one edge in a phase makes arbitration "a provable no-op". So the useful
question is not "how do we do it once" but:

> **What fraction of edges, in a real frame, have `scale == 1.0`?**

In the interior of a settling pool — cells neither near-empty nor near-capacity, with modest
fluxes — the answer should be "nearly all of them". If it is, the win is a cheap early-out test
(compare two sums against two budgets) that skips the scaling arithmetic and, more importantly, may
let COLLECT and APPLY fuse for uncontested edges, since the three-pass split exists *only* because
arbitration needs all claims before any application.

**This is the measurement to take first**, and it is a counter, not a profile — immune to the
symbol-attribution problem that has produced two wrong conclusions already. It also directly sizes
the prize: if 90% of edges are uncontested, the three-pass structure is being paid for the 10%.

---

## 3. The n-step question: the literature has a specific, conservative answer

This is **local time stepping (LTS)** for conservation laws, and the multi-rate scheme we have
invented by hand is a known object with known failure modes.

**Osher & Sanders (1983)** is the origin: advance the fast cells with fractional steps, then **sum
the fluxes across the shared interface over all fractional steps, and use that sum as the flux seen
by the slow cell**. Conservation is exact because the identical quantity leaves one side and enters
the other — it simply happens at different times. Dawson & Kirby extended it to second-order
Runge-Kutta; Tang & Warnecke showed some formulations lose consistency and repaired it. The standing
difficulty in that literature is *high-order accuracy* with conservation, not conservation itself,
which is solved.

**This matters here because it replaces the mechanism that just cost us a correctness bug.**

Our current approach to a clock-domain boundary is **neighbour forcing**: a fast block force-wakes
its slower neighbours so the shared edges get evaluated. That is what `force_overclocked_blocks_active`
does, and it is the source of both defects we hit:

- **The S3 violation** (`MASS-ERR-DIAGNOSIS.md`): when a fast block settled mid-frame it stopped
  forcing, its slow neighbour's owned edge went unevaluated, and mass redistributed across the
  boundary — measured as two adjacent block rows with opposite-signed, magnitude-matched excess.
- **The cost**: forcing pulls the neighbours in, which is why early stop's 1.5x evaporated once
  forcing was honoured. Blocks run went 297 -> 489 against 492 with early stop off entirely.

**Under Osher–Sanders the slow block does not run at all.** The fast side accumulates its interface
flux over its sub-steps into a buffer; when the slow block takes its own step, it applies the
accumulated total. Consequences, all of them things we currently lack:

- Conservation at the boundary holds **by construction**, not by scheduling discipline. The S3
  hazard stops being a hazard — there is nothing to forget to force.
- **No forced neighbour runs**, so the repetition count actually falls with the clock rate. Early
  stop's saving becomes collectable, because the reason it was not collectable was forcing.
- It composes with arbitrary (non-power-of-two) rates, which we already have, because it makes no
  assumption about nesting — only that the accumulated flux is applied when the slow side steps.

**The catch, and it is a real one.** Osher–Sanders assumes the slow cell's state is *frozen* during
the fast cell's fractional steps (its values at fractional levels are taken as the value at `t_n`).
Our fast side's flux depends on the slow side's `h` through the equilibrium solve, so accumulating
against a frozen neighbour is an approximation whose error grows with the rate ratio. That is
exactly the accuracy-versus-conservation tension the literature describes. Conservation is still
exact; *accuracy* degrades. Given this project's priorities — mass conservation is non-negotiable,
visual plausibility is the bar, and `mass_err` is already the tightest invariant we hold — that is a
favourable trade, but it should be stated rather than discovered.

**Second catch: arbitration must see the accumulated flux.** A slow cell receiving 8 sub-steps'
worth of accumulated flux in one application can be driven past capacity or below zero by a claim
that was legal at each sub-step. The FCT limiter must therefore be applied to the *accumulated*
claim at the moment it lands, not only to each sub-step's contribution. This is the place a naive
implementation would silently break the non-negativity guarantee, and it is worth writing into the
design before anyone builds it.

---

## 4. Per-edge early termination is exact where per-block early stop was not

Block-level early stop failed because "this block has stopped moving" was conflated with "this block
need not force its neighbours". The per-*edge* version has no such conflation: an edge whose
equilibrium transfer returns zero moves no mass this step, and skipping its APPLY is exactly
equivalent to performing it.

The pairwise solve converges geometrically to the pair's equilibrium, so in a settling region most
edges go quiet within a few sub-steps while a moving front stays active. This is the same
59-62%-of-work observation the profile made, relocated to a level where acting on it does not
violate an invariant.

**It does not, by itself, save the arbitration pass** — a zero-flux edge still contributes zero to
the totals — which is why §2's uncontested-edge test and this are complementary rather than
alternatives.

---

## 5. What I would measure before building anything

In priority order, all counters rather than profiles:

1. **Fraction of edges with `scale == 1.0`** (uncontested), by material and by scene phase. Sizes
   §2 directly.
2. **Fraction of edges whose transfer is zero** at each sub-step index, 1..8. Sizes §4, and says
   whether edge-level quiescence really concentrates the way block-level did.
3. **Interface flux magnitude across clock-domain boundaries** versus interior flux. Sizes how much
   the Osher–Sanders frozen-neighbour approximation would actually distort, before committing to it.

None of these needs the sampling profiler, which is the point: this project has had two wrong perf
conclusions from symbol attribution and none from counters.

---

## Sources

- [An efficient local time-stepping scheme for solution of nonlinear conservation laws](https://www.sciencedirect.com/science/article/abs/pii/S0021999110004304)
- [On some explicit local time stepping finite volume schemes for CFD](https://www.sciencedirect.com/science/article/abs/pii/S0021999119305029)
- [Enforcing the Courant-Friedrichs-Lewy Condition in Explicitly Conservative Local Time Stepping Schemes](https://arxiv.org/pdf/1801.03108)
- [Time adaptive conservative finite volume method](https://www.sciencedirect.com/science/article/abs/pii/S0021999119307727)
