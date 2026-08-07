# #70 — 2.47 — DESIGN DIRECTION: replace the equilibrium head field with per-cell OVERFILL. Pressure becomes stored state, free fall and upward relief stop being special cases

**Status:** pending

---

USER DESIGN, 2026-08-07, and it is the strategic direction the head-field work should hand off to. Their words: "maybe we need to find a answer for pressure that works with per cell propagation fast and an answer for falling liquid that is not special case. same for material transportation, we have to do per cell. so we have to figure out how to achieve it from first principle. optimistic material movement targeting 1 but allowing 1.2 but next round force it to spread those around?"

And, on relief: "we need to allow upwards movement for overfill. so that everything can't be 1.2 but we would probably see a gradient."

## The proposal

Let a transfer overshoot. A cell's capacity is 1.0 but it may transiently hold more (1.2 as an emergency ceiling). The excess is not discarded and not rejected -- it is redistributed on subsequent ticks. This is artificial compressibility / position-based-fluids in shape: violate the constraint, then relax it back, rather than testing feasibility per edge against a frozen state and throwing away everything that does not fit.

## Why this is worth doing -- it dissolves three problems that look unrelated

**1. The saturated interior becomes mobile.** MEASURED this session: a levelling step produces IDENTICAL flow at 10 and 20 cells deep (133.60 both, toggle on and off). In a full body every interior acceptor is at capacity, so `flux_edge_candidate` clamps those edges to zero however much head the donor carries, and the mobile set is a one-cell surface skin. That is a bookkeeping artifact, not physics: the clamp is evaluated against the FROZEN pre-tick state, so it cannot see that the acceptor is about to give the same mass away. A real full pipe conducts flow perfectly well because it is a CHAIN of simultaneous transfers. Overfill makes the chain expressible -- A to B this tick, B to C next.

**2. Overfill IS pressure.** A cell at 1.2 against a capacity of 1.0 carries 0.2 of excess, which is exactly "how hard is this being squeezed". Per-cell, local, no propagation sweep, no max operator, no connectivity graph. Crucially it is CONSERVED AND ACCUMULATING rather than recomputed from scratch -- which is the difference from `column_depth` (local but memoryless, so it could only ever see straight up).

**3. Free fall stops being a special case, entirely.** A falling cell has nothing beneath it pushing back, so it accumulates no overfill, so its pressure is zero. Zero pressure EMERGES from "nothing is resisting" instead of being asserted by a support predicate. #67 and #69 are both failures of that predicate in opposite directions; both evaporate if the predicate does not need to exist.

## Upward relief is the SAME rule, not an added case

Do not add "overfill may move up". Add overfill to the driving term and stop letting gravity be the only one:

    driving = gravity + (overfill_below - overfill_above)

Flux goes up whenever the overfill gradient beats gravity locally. Falls out.

This is also the mechanism that finally connects "there is pressure at depth" to "the surface moves" -- push water into the bottom of a full vessel and the surface should rise. That is impossible today (rigid interior), and it is exactly what #64's unfilled valley needed.

## "Everything ends up at 1.2" is a tuning failure, not the design

USER CONCERN, worth answering directly because it is the natural objection: "so that everything can't be 1.2 but we would probably see a gradient. not what we want ideally."

The equilibrium overfill is NOT 0.2. It is whatever makes the overfill gradient exactly cancel gravity. With stiffness `k` (how hard overfill pushes back per unit excess), at depth `d`:

    overfill(d) ~= gravity * d / k

Make `k` large and the deepest cell of a 300-row column sits at 1.01 while the surface sits at 1.000. **1.2 is a safety valve for impact transients, not an operating point.** In steady state it should never be reached.

And the depth gradient is not a defect to be engineered away -- it IS the hydrostatic pressure field, arrived at locally. That is the prize, not the cost.

## `k` is DERIVED, not tuned -- this is the first-principles hook

Require hydrostatic equilibrium to be a fixed point: at rest the overfill gradient force must exactly cancel gravity. Then choose the maximum overfill tolerable at the deepest point of the deepest scene (say 2%) and that FIXES `k`. One equation. No sweep, no fitting a constant until a test goes green -- which matters given how much of this project's history is exactly that failure mode.

`k` also sets the acoustic speed, i.e. how fast pressure propagates, against a tighter CFL bound. Same trade the head field's sweeps made, now local and explicit.

## Why this survives the adaptive block scheduler and the head field does not

This may be the strongest architectural argument, and it explains a defect we spent the session chasing.

A globally-coupled INSTANTANEOUS solve is fundamentally incompatible with a scheduler that skips regions. The head field is recomputed from scratch each tick as an equilibrium over the whole connected body; equilibrium is a global statement, so if half the domain did not advance, the answer is computed over a mixture of current and stale state and there is no principled partial version.

Overfill is STORED STATE, per cell, conserved. A sleeping block simply does not relax this tick; its excess sits there still exactly correct and relaxes when the block next runs. Nothing stale, nothing lost, and the result does not depend on WHEN blocks ran. Same property the frozen-Jacobi conversion bought on tendrils: order-independence comes from transporting a conserved quantity instead of deriving one.

A partially relaxed overfill field is a VALID STATE. A partially converged equilibrium solve is garbage. See #68.

## USER DECISIONS, 2026-08-07 -- treat as settled

1. **Some slosh is accepted.** Upward overfill transport plus gravity is a surface gravity wave and it will slosh. That is real physics and the user has explicitly accepted it. Do NOT damp it away as a bug. Note `wave_params`' damping was calibrated for surface-only motion and becomes load-bearing in a new way.
2. **Overfill does NOT render.** Draw `min(h, cap)`; the free surface stays flat and the ~1% is invisible. Consequence: mass and height diverge, so every metric must declare which one it means. The surface cell is where it genuinely matters -- a surface cell at 0.4 really is 0.4 full.
3. **The pressure heat-map shows overfill.** That becomes the overlay's source, replacing `column_depth` / `head_field_to_pressure`. It is also the debugging instrument for this whole scheme -- a striped or banded overfill field is visible immediately.

## What happens to the head field

KEEP IT, as an ORACLE rather than a driver. It computes the correct equilibrium answer cheaply and verifiably (2 sweeps; an off-centre column read exactly 10.00 and 20.00 rows of head where it should). That makes it the ideal acceptance test: run the local overfill scheme to steady state and compare against the head field's answer. A real convergence criterion instead of a visual check. The thing that failed as a transport driver is genuinely good as a reference.

## Risks and open questions

- Stiffness is a spring; too stiff and it rings, and that ringing would look exactly like the striping in #68. Relaxation rate and overfill allowance are ONE parameter, not two.
- What if a whole region reaches the ceiling? Unbounded-but-penalised overfill is more likely to stay stable than a hard cap, since a hard cap reintroduces the rejection this design exists to remove.
- Mass conservation must stay exact through the redistribution step. Non-negotiable; every existing conservation test applies.
- Granular gets a natural home here rather than a bolt-on: "resists redistribution below a threshold gradient" IS the angle of repose, and it is the yield criterion the head field never had. Do not assume it transfers for free, but it is the right shape.

## First steps, in order

1. **Measure the ceiling first, before building anything.** Instrument what fraction of wet cells are currently clamped to zero flux by the acceptor test in a typical scene. If it is the surface skin only -- which the 133.60-at-both-depths result implies -- that number is the hard ceiling on everything pressure work could ever have bought, and it is the strongest single argument for spending the next block of work here. Cheap: a counter in `flux_edge_candidate`'s call sites.
2. Derive `k` from the hydrostatic fixed point and a chosen maximum overfill. Write that derivation down before coding it.
3. Prototype behind a toggle, same pattern as every other physics change here (sim field -> `settle_tick` parameter -> wasm setter -> demo.js -> checkbox, AND its change listener -- see the handover).
4. Validate against the head field as oracle on the existing static specs.

## Cross-links

#55 (the head field -- becomes the oracle), #67 and #69 (both are the support predicate failing; both should evaporate), #68 (the scheduler-incompatibility argument comes from here), #64 (needs the rigid interior fixed, not a better driving head), #54 and #33 (the original "make pressure drive every flow" and "sideways movement" asks this actually answers), #45 (the 45-degree cone -- same 1-cell-per-tick limit, but a compressible acoustic speed is an honest answer to it rather than a workaround).
