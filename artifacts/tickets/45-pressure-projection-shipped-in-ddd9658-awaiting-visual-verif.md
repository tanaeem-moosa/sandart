# #45 — 2.22 — Pressure projection: SHIPPED in ddd9658, awaiting visual verification

**Status:** completed

---

USER-CONFIRMED VISUALLY after 21ca3843. A 45-degree feature forms where sand is fed from the top. User's own diagnosis: "probably because of pressure propagation" — correct.

## USER DIRECTION 2026-08-04: COMMIT TO THE FIRST-PRINCIPLES FIX, DO NOT GATE IT ON REPRODUCTIONS

"I think if we over invest on trying to recreate all the issues in test, we will never get anywhere. sometimes we need to commit to first principle solution. pressure is one of those. it will fix more issues and open opportunities for fixing more issues than it introduces. so let's get the pressure right."

Binding on how this ticket is worked. Do NOT build a reproduction or a metric for each open artifact before starting. The only HARD GATES are the correctness invariants that already exist: mass conservation structural (1e-9..1e-8 measured band, not the loose 1e-4 assertion) and exact positivity. Everything else — grid-scale parity power, the #52 mirrored-pair left-bias measurement, the neck-sweep drain-order diagnostics — is an AFTER-THE-FACT observation to see what the projection moved, not a prerequisite.

## WHY 45 DEGREES IS THE DIAGNOSTIC

An explicit scheme propagates influence ONE CELL PER TICK in each direction, so its numerical domain of dependence is exactly a 45-degree cone. Real pressure is elliptic and propagates across the whole connected fluid instantly. A 45-degree feature is that characteristic cone made visible — not a shape in the physics.

CONFIRMATION: it appeared in WATER first, and sand joined it the moment Stage C put sand on the same solver. Same solver, same cone, both materials. Not tunable; will not yield to ordering, arbitration, or scan-order work — several of each have been tried and measured.

## AGREED DESIGN 2026-08-04

User: "as for how to pressure, I was expecting on the edges. and using it for stoacastic proportional contribution."

### Pressure lives on edges

Pressure at cell centres, gradient and flux on FACES, divergence assembled from face fluxes. Gradient at an edge is the ONE-CELL difference `p[i+1] - p[i]`, NEVER `(p[i+1] - p[i-1]) / 2` at a cell centre.

This is not stylistic. On a collocated grid, centred differences link cell i to i±2 and never to i±1, so odd and even sublattices decouple and a checkerboard pressure field sits in the operator's null space — invisible to the solver, fully visible on screen. The negative tendrils (below) are already a near-constant-pitch grid-scale mode, so a collocated projection would AMPLIFY an existing defect and look like the pressure work caused it. The existing edge-flux solver is already a staggered/MAC arrangement; keeping it means no null mode and no Rhie-Chow interpolation needed.

### Stochastic proportional contribution — the machinery already exists

`edge_share_jitter` (physics.rs:743) is already "a stateless hash of `(time_seed, edge key, salt)` rather than a stored RNG stream", and its doc at :657 says it exists to "divide a contested budget with randomised weights instead of equal ones". The work is re-pointing it at the pressure gradient, not building it.

Two properties to protect explicitly through the rewrite:

1. **Stateless hash, not a stream.** The adaptive scheduler drops blocks, so a sequential RNG would draw a different number of times per tick depending on the frame budget — every cell's noise would shift with frame timing and the sim would stop being reproducible between runs or machines. Keying on `(edge_key, salt, time_seed)` makes that structurally impossible. physics.rs:785 notes APPLY re-derives the same jitter from the same key; that is what keeps the two-phase propose/apply consistent.
2. **Randomised WEIGHTS, not coin flips.** Jitter the shares of a proportional split so the split stays renormalised and conservation stays structural. "Transfer with probability p" per edge conserves only in expectation and would break the 1e-9..1e-8 band.

### Keep the solve deterministic; put the stochasticity in the application

The Poisson relaxation must converge deterministically. Pressure is elliptic and the whole point is propagation across the connected region within a tick; noise injected into the relaxation fights convergence and gives back the one-cell-per-tick behaviour being paid to remove. Randomise the APPLICATION of the converged edge fluxes.

### What this should and should not fix

SHOULD: a checkerboard mode and a systematic lean are both COHERENT structures needing consistent phase across many cells and ticks. A per-edge, per-tick hash re-randomises that phase, so neither can accumulate.

SHOULD NOT: jitter randomises the SHARE, not the SIGN of a driving term. A bias living in the equations — e.g. the `head_a`/`head_b` inconsistency at a water|sand edge in #52 — will survive. If the left bias persists after this, that is where it is.

## WHAT IT SHOULD ALSO FIX

- Remaining drain-order gap: sand f_50 0.587, liquid 0.644, ideal 2*m_black = 0.774. Arbitration closed most of it; propagation is what is left.
- Subtle NEGATIVE tendrils at the TOP drifting LEFT (#44). Reconfirmed 2026-08-04: "that is what I called negative tendrils. they seems to move. I think pressure will fix it." They appear in the MultiNeck photo as a fine comb of near-constant pitch (crop `artifacts/design/lines/left_pile.png`). "They seem to move" is what rules out any display artifact — a real advected structure. Constant pitch at the grid scale = a wavelength set by the discretisation, hence the edge-vs-collocated constraint above.
- POSSIBLY the row-axis twin: `diag_falling_block_slab_separation` measured 2-cell-wavelength ROW banding (row parity 0.257 vs column 0.002 vs checkerboard 0.006), attributed to odd-even decoupling of the simultaneous update. Same shape on the other axis. NOT ESTABLISHED as one cause — different scenarios, different quantities.
- The #52 vertical water/sand front may be partly this, but #52 also reproduces in a flat side-by-side setup with no funnel, so do not assume it closes.

## ADDED 2026-08-02: upward motion for splashes

User: "maybe with pressure we need to let water move up a little to create splashes".

Water currently cannot move against gravity, so an impact has nowhere to put its momentum and there is no splash. A pressure projection is the natural place: high pressure under an impact should push liquid UP through the same gradient that pushes it sideways, rather than upward motion being bolted on.

Design it in from the start — a projection that structurally forbids upward flux would have to be reworked later. Relevant to #27 (water towers/splashes violently).

Constraint: upward flux must not break capacity/positivity or let liquid climb indefinitely. A pressure-gradient response, not buoyancy.

## APPROACH

Pressure projection / Poisson solve so pressure propagates across the connected region within a tick. Iterative relaxation is the tractable form. The existing frozen-Jacobi + capacity-arbitration structure is the right substrate — the user chose Jacobi partly because it "may enable pressure simulation easier", which turned out correct.

## SEQUENCING

Next up. Agreed 2026-08-02: pressure (this) -> #47 slabs -> #49 acceleration -> #44 symmetry.

Sand now shows liquid's left-drift asymmetry too, so the cause is in shared machinery a projection would substantially rewrite. Working #44 first risks doing it twice. Two fixes for #44 have already failed (red-black edge colouring, column_depth freeze) — both removed order dependence WITHOUT removing the lean, so the lean is order-independent and unexplained.

## EXISTING MEASUREMENT (for after, not before)

- `diag_step1_mass_vs_core_flow_*` — f_50 and white_frac@10% across the neck sweep, absolute geometric reference (ideal f_50 = 2*m_black, cumulative metric).
- `diag_step1_phase_capacity_attribution_*` — phase-attributed flux and free-capacity split.
- Both sweep neck_width [0.02, 0.04, 0.08, 0.12]; the defect is outlet-size dependent and a single wide-outlet point is blind to it.

## QUOTA NOTE
Weekly cap reset 2026-08-04 21:00. Large piece of work; start on a fresh week.
