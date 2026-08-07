# #62 — 2.39 — Warm-start the head field with a decay term so switchback geometry does not re-derive from cold every tick

**Status:** pending

---

FUTURE OPTIMISATION, not a defect. The head field (`task55_head_field::advance_head_field`) is currently COLD-SEEDED every tick: every wet cell is overwritten with its own local hydrostatic head, and the previous tick's values are never an input. It then max-propagates to convergence within the call.

WHY IT IS COLD TODAY. `max` is monotone, so if a cell took the max against its own previous value the field could only ratchet upward and would never fall when a reservoir drains. Cold seeding is what makes a DROP propagate exactly as fast as a RISE, and it makes the field a pure function of geometry (no hysteresis, no dependence on material flow).

WHY WARM MIGHT STILL WIN. A sweep carries a value arbitrarily far ALONG the sweep direction, so cost is the number of DIRECTION REVERSALS in a connectivity path, not its length. Measured today: 2 sweeps for every scenario in the spec harness, U-tube included. But a serpentine/switchback channel (up-down-up-down, as a procedural cave can generate) needs roughly one sweep per reversal, so 3, 4, or more. `HEAD_FIELD_SWEEPS_PER_TICK = 32` covers that, but re-deriving the whole field from cold each tick is wasted work when only a small differential actually changed.

THE DESIGN. `seed[i] = max(own_local_hydrostatic[i], prev[i] - decay)`. Note this is a KNOB, not a different algorithm: as `decay -> infinity` it reduces exactly to today's cold seed, and as `decay -> 0` it is a pure warm start. So this task is about choosing a point on that line, with the cost stated:

- Falls become rate-limited at `decay` per tick while rises stay instant. A drop of D takes D/decay ticks to bleed out. Draining vessels and breaking siphons are exactly the cases that would lag.
- Hysteresis returns; the field stops being a pure function of geometry.
- The per-tick convergence SIGNAL is lost. Today the sweep loop either exits early or hits the cap, so "did this tick converge?" is answerable. That check is what a `debug_assert!` (compiled out in release) failed to provide when a barely-relaxed field shipped silently.
- `decay` has no physical anchor, so it becomes a tuned constant. Do not pick it by trying values until a spec passes.

DO NOT START THIS WITHOUT A MOTIVATING MEASUREMENT. Build a switchback/serpentine scenario (or use the procedural cave generator) and measure actual sweeps-to-converge at w=512. If it stays in the low single digits, this task should be closed as unnecessary rather than implemented. The number to beat is 2.

Raised by the user 2026-08-07: "I can imagine a complex geometry where we may need 3 or 4 pass, right? like a up down up down and so on. so there is some advantages of keeping a history - decay term. can be a future task."
