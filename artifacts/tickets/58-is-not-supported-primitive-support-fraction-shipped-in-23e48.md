# #58 — 2.35 — "is not supported" primitive: support_fraction SHIPPED in 23e48e9; pressure reuse still open

**Status:** completed

---

USER DIRECTION 2026-08-05: "let's add is not supported signal for this reason. but is not supported is what I thought we needed for pressure too so we can figure out how to utilize that later."

## The insight

`in_transit_at` conflates two different questions and answers both with a velocity proxy:

- **Is this material MOVING?**
- **Is this material SUPPORTED?**

They are not the same. Supported material transmits load whether or not it is moving — that is the basis of Janssen and Beverloo, and it is why a packed column squeezing through a neck still presses on what is below it. The zero-load case is FREE FALL, i.e. unsupported, not "in motion". (This was established 2026-08-05 after the user challenged an earlier claim of mine that sustained flow should carry no pressure. It should.)

Both consumers actually want SUPPORT, not motion.

## What it is

A direct state predicate:

- A cell is SUPPORTED to the extent that what is below it can bear it — the cell below at or near capacity, or wall / outside the mask.
- It is UNSUPPORTED to the extent that there is FREE CAPACITY below it.
- Graded in [0,1] rather than boolean if that costs nothing extra. Partial support is real, and a graded signal is more useful to the pressure path than a hard flag.
- NO dependence on `edge_vel_v` and NO dependence on the previous tick.

That last property is the important one. Because it is pure state:

- it can wake a block BEFORE anything moves, which every previous activation signal could not (they all read `last_displacements`, which is one tick behind — the root cause of the slab defect);
- it cannot go stale in a block the scheduler skipped, unlike anything derived from per-edge velocity.

## Consumer 1, being built now: block scheduling (#47)

Replaces the near-zero-overburden clause in the slab predicate.

Why the overburden clause was the wrong signal: it depends on `in_transit_at` zeroing out for falling material, and `in_transit_at` UNDER-DETECTS free fall. Measured elsewhere: a column at `h` = 0.77-1.00 that drained to 0-0.19 within the SAME tick reported `in_transit` of only 0.0-0.55. That leaves `resting_above = h - in_transit` at roughly 0.4-0.5 per row instead of ~0, so accumulated overburden crosses the predicate's 1.5 threshold after about three rows. The predicate could therefore only ever see the top sliver of any falling body — consistent with it promoting 2-10% of blocks against perfect-simulation's 20-33%, and with its divergence improvement being only 8.4% cumulative / 14% peak.

The user's own reasoning was right and the sensor was wrong: "a slab happens when the block below it is in free fall, so it should work."

## Consumer 2, later: the pressure term (#55, #57)

`resting_above = max(0, h - in_transit_at - external)` exists to stop unsupported material contributing overburden. A support predicate answers that question directly instead of inferring it from speed.

DO NOT wire it into the driving head as part of the scheduling work. That is a separate decision and it interacts with the additive-versus-multiplicative rework in #55 — under the multiplicative free-surface form the whole shape of the depth term changes, so adopting this there should be designed with that, not bolted on first.

## Design requirement

Put it where both callers can reach it, name it for what it MEANS rather than for its first caller, and document the intended second use in place.

## Cross-links

#47 (slabs — the immediate consumer), #55 (the elliptic head solve and the multiplicative free-surface form — the eventual second consumer), #57 (the arch, where `in_transit_at`'s behaviour was first measured), #54 (the vertical overburden bonus, additive and due for rework under #55).
