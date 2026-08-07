# #56 — 2.33 — ASYMMETRY HUB. Check the DISPERSION term first: zero-mean by construction but drawn from a non-uniform hash, added to one edge endpoint only, and scaled by the YIELD THRESHOLD

**Status:** pending

---

REWRITTEN 2026-08-07 after the user photographed the defect and supplied the constraint that kills most of the previous hypotheses. This is now the hub ticket for asymmetry; #44 is the water-specific case and probably has a DIFFERENT cause (see below).

## READ THIS FIRST: two different phenomena have been conflated

- `test_water_blob_stays_left_right_symmetric_under_gravity` measures `worst = 1.4e-5`, `final = 2.0e-6`, and fails on PERSISTENCE (75 consecutive same-sign ticks against a bound of 25), not magnitude.
- What the user photographs is MACROSCOPIC: a visibly one-sided pile, at w=256 and w=512.

These are almost certainly not the same bug. Five fixes have been aimed at the 1e-5 signal. The standalone `column_depth` pass cut THAT signal 4x and never moved the visible drain asymmetry -- which is exactly what you would expect if they are separate. STOP USING THE BLOB TEST AS THE INSTRUMENT FOR THE VISIBLE DEFECT. It is not measuring it.

## The constraint that narrows everything (USER, 2026-08-07)

MultiNeckHourglass, 3 necks, Sand-fall, reproduced at w=256 and w=512: "always same [direction]. and with mutlineck same for each neck creating this pile on one side."

**Identical bias at every neck rules out geometry.** A container off-centre by a cell is a GLOBAL perturbation: three necks at three different x positions would be biased by different amounts, and barely at all near the middle. Identical bias at all three means the cause is LOCAL and TRANSLATION-INVARIANT -- something that treats +x and -x differently at every cell regardless of position.

Also RULED OUT by the same observation: floating-point non-associativity. The user's judgement, and it is right -- f32 rounding cannot produce a visible lobe.

## PRIME SUSPECT: the dispersion term. Check this before anything else.

`physics.rs`, lateral edge site (~4694):

    let disp_roll = ((seed ^ (nb_idx as u32).wrapping_mul(823)) & 0xFF) as f32 / 255.0;
    let dispersion = (disp_roll - 0.5) * 2.0 * DISPERSION_TAU_FRAC * tau;

where `seed = x*1299689 ^ y*314159 ^ time_seed*7213`.

Three properties, and it is the COMBINATION that matters:

**1. It is added to ONE ENDPOINT ONLY.** All three driving-head branches build `head_a = ... + dispersion` and `head_b` without it. Each cell owns the edge to its RIGHT, so `a` is always the left cell of every lateral pair, everywhere, every tick.

**2. It is not actually random -- it is a deterministic, poorly-mixed hash of x.** `(disp_roll - 0.5)` is zero-mean IF the low byte is uniform. It is not obviously uniform. Taking low 8 bits: `1299689 mod 256 = 233` and `823 mod 256 = 55`, so the byte is `(233x + A) XOR (55x + B) mod 256` with `A`, `B` fixed by `y` and the tick. **XOR of two affine sequences in x is not uniform and its mean over x has no reason to be 127.5.** Low bits of a multiply are the classic place for this to go wrong.

**3. It is scaled by `tau`, the YIELD STRESS.** This is what makes a small bias macroscopic. Dispersion is not a nudge on the flow RATE -- it is comparable in size to the threshold that decides WHETHER material yields at all (`driving > tau` versus `driving < -tau`). A systematic bias of a few percent of `tau`, applied to one side of every edge, biases which direction crosses the yield criterion. That is a decision, not a magnitude, and decisions compound.

**It is granular-only.** `tau = GRANULAR_TAU_SCALE * threshold_prop * granular_share`, which is 0 for liquid, so `dispersion` is identically zero for water. That MATCHES the user's sand repro exactly, and it means this CANNOT explain #44's water drift. Two causes, two tickets.

### The check, and it is arithmetic rather than an experiment

Compute `mean over x of dispersion(x, y, t)` for a spread of `y` and `t`, directly from the formula. No grid, no ticks, no simulation, no chaos. If that mean is consistently nonzero with a stable sign, the bug is found with certainty.

If it is nonzero: the fix is NOT to re-tune `DISPERSION_TAU_FRAC`. Either draw from a properly mixed hash (use HIGH bits, or a real integer mixer such as a splitmix/murmur finalizer), or -- better and cheaper to reason about -- **split it symmetrically across the edge**: `head_a += dispersion/2` and `head_b -= dispersion/2`, so the term perturbs the DIFFERENCE without favouring an endpoint. That is correct regardless of whether the hash is biased, and it removes the whole class of defect.

## SECOND CHECK, once dispersion is settled: is the edge function antisymmetric?

A unit test on `flux_edge_candidate` alone. No grid, no ticks. Call it with `(head_a, head_b, cap_a, cap_b, avail_a, avail_b, h_a, h_b, v_e_prev)` and again with every pair swapped and `-v_e_prev`; the result must be the EXACT negation.

If that holds, the function is clean and every remaining bias lives in the CALLER -- i.e. in how `head_a` and `head_b` are built, which is where dispersion already demonstrably breaks it.

## REMAINING candidates, all local and translation-invariant, all findable by inspection

- **Edge ownership in arbitration.** Every cell OWNS its right edge and merely participates in its left neighbour's. If the rescale walks owned edges differently from borrowed ones, or if `cell_avail`/`cell_freecap` are written by the owner and read asymmetrically, that is a per-cell directional bias with exactly the right signature.
- **Tie-breaks.** `<` versus `<=` on `h_a` against `h_b` resolves every exactly-level pair in the same direction. A settled bed is full of exact ties.
- **`column_depth` reading LIVE `temp_heights`** and chaining off its own earlier values in the same pass. Known, documented, and the reason the standalone pass cut the small signal 4x -- but note it is x-order-dependent, not direction-biased, so it is a better explanation for the 1e-5 test than for the visible pile.

## What NOT to do

- Do not re-tune tolerances on `test_water_blob_stays_left_right_symmetric_under_gravity`, ignore it, or retitle it. It is a deliberate marker (see the handover) and it is measuring a real, separate, smaller thing.
- Do not try another SCAN ORDER. Hilbert and diagonal orders were tried and only relocate the bias, and there is a principled reason: alternating direction makes the TIME-AVERAGE symmetric, but the solver is nonlinear (yield threshold, donor/acceptor clamps, edge sleeping) and clamping is irreversible, so averaging two asymmetric operators does not produce a symmetric one. Order ROTATION cannot work; only order ELIMINATION can.
- Do not reach for another trajectory-level experiment. Five have been run and each returned one ambiguous scalar. Everything recommended above is either arithmetic on a formula or a property of a pure function -- none of it can come back inconclusive.

## Original framing, still correct and still open

USER POINT 2026-08-05: "pressure is symmetric and it is naturally against assymetry so it makes sense that it reduces assymetry. but it is still a bandaid on underlying assymetry issues that we need to find at some point."

Moving `column_depth` into a standalone unconditional pass (#54 step 1) cut the tick-phase symmetry error ~4x and nearly converged the even/odd runs. That is damping, not a fix, and the 4x is on the SMALL signal only.

## Cross-links

#44 (water drains asymmetrically -- must be a different cause, since dispersion is zero for liquid), #54 (the standalone pass), #45 (pressure projection, which also damps), #70 (if the overfill rewrite happens, the yield criterion moves and this term's role changes -- re-check then rather than assuming it carries over).
