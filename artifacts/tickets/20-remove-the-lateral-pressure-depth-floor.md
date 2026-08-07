# #20 — 2.2 — Remove the lateral-pressure depth floor

**Status:** completed

---

`LATERAL_PRESSURE_DEPTH_FLOOR = 1.5` (physics.rs:272) is a deadband, not physics. RESOLVED 2026-07-29 — direction chosen, see "Decision".

## Findings (2026-07-29, at b6246ae)

**1. The app cannot produce the phantom.** There is NO external mass injection path anywhere in the product. The brush is `displace_line` (physics.rs:680) — "carving a groove and depositing the displaced volume into the surrounding ridge area", volume-conserving displacement. The entire wasm API surface (sandart-wasm/src/lib.rs) has no pour/add call; mass enters only via `reset` / `initialize_hourglass`, one-shot. Only the TEST HARNESS injects, by writing `hm.data[..] = 1.0` every tick.

**2. Exactly one test depends on the floor.** Set `LATERAL_PRESSURE_DEPTH_FLOOR = 0.0` and run the full suite: **77 pass, 1 fail — `test_liquid_stream_stays_coherent`**. Nothing else moves. So the floor's entire cost (400-tick enclosed-void total 21938 vs 12106 at floor 0, ~45% of the remaining standing-in-walls behaviour) is paid by every water simulation to satisfy one test whose stimulus the app cannot generate.

**3. It is probably visible.** See #27 (2.4) — water "towery with a large neck" on the deployment. Standing-in-walls is exactly what the void metric counts.

## Decision: fix the estimator, not the test
Chosen over changing the test's stimulus because a WATERFALL FEATURE is wanted later, and that needs a genuine continuous external source. Making injection first-class is the architecture that feature requires anyway; the test then keeps its infinite tap, its rate and its assertions unchanged, and only the WRITE MECHANISM becomes visible to the solver.

Fallback if that proves gnarly: apply the floor only to externally-written cells — keeps both the tap test and a zero floor everywhere the app actually lives. Do NOT fall back to changing the test's source to a draining reservoir.

## Why the phantom happens
Lateral driving head is `h_a + LATERAL_PRESSURE_SCALE * max(column_depth - LATERAL_PRESSURE_DEPTH_FLOOR, 0)`. `column_depth` = "cells of RESTING liquid stacked above me", computed top-down as sum of `(h - in_transit)`. Falling liquid must read ~0 so a stream does not push itself apart, and in a stream's INTERIOR the `in_transit_at` subtraction (physics.rs:338) cancels almost exactly.

It fails at the SOURCE. Source cells are refilled from outside each tick, so their content is NEW liquid that has not begun falling — it is not resting overburden, but `h - in_transit` reads it as full. Cells below inherit a few cells of phantom resting depth. The floor is a deadband sized to swallow that.

## Floor sweep (at the shipped scale = 5.0), for reference
  floor  max_width  void total
  0.0       11  FAIL   12106
  1.0       10  FAIL   19998
  1.4        8         21769
  1.5        8         21938   <- shipped
  3.0        8         26423
Lower is monotonically better on walls. Note 12106 was measured WITH the phantom present; after the fix the number will differ — report the real one.

## Must not regress
- All 78 tests pass (78 at b6246ae, not 77 — the merged material-preset PR added one).
- `test_liquid_stream_stays_coherent`: max_width <= 8, peak_h >= 0.5. Assertions unchanged.
- Sand bit-identical — every path here is gated on `cell_liquidity > 0.0`.
- Water ~1.36 ms/tick at budget 1024.
- Bench `mass_rel_err` ~1e-9.
- Do NOT weaken any assertion; report numbers instead.

## Unblocks
#26 (2.3) — `tau` on the lateral edge would otherwise be tuned against this same driving term.
#27 (2.4) — symptom A may close with this.
