# #56 — 2.33 — ASYMMETRY HUB. Randomness is probably NOT the cause (math recorded). Salt experiment first, then edge OWNERSHIP and arbitration, which never alternate

**Status:** pending

---

REWRITTEN TWICE on 2026-08-07. The second rewrite demotes the randomness hypothesis after the user pushed back and turned out to be right. Read the "randomness is demoted" section before spending any time on the hash -- the argument against it is arithmetic and it is recorded here so nobody re-derives it.

## READ THIS FIRST: two different phenomena have been conflated

- `test_water_blob_stays_left_right_symmetric_under_gravity` measures `worst = 1.4e-5`, `final = 2.0e-6`, and fails on PERSISTENCE (75 consecutive same-sign ticks against a bound of 25), not magnitude.
- What the user photographs is MACROSCOPIC: a visibly one-sided pile, at w=256 and w=512.

These are almost certainly not the same bug. Five fixes have been aimed at the 1e-5 signal. The standalone `column_depth` pass cut THAT signal 4x and never moved the visible drain asymmetry -- exactly what you would expect if they are separate. STOP USING THE BLOB TEST AS THE INSTRUMENT FOR THE VISIBLE DEFECT.

## The constraint that narrows everything (USER, 2026-08-07)

MultiNeckHourglass, 3 necks, Sand-fall, reproduced at w=256 and w=512: "always same [direction]. and with mutlineck same for each neck creating this pile on one side."

Three things follow, and together they are very restrictive:

1. **Geometry is ruled out.** A container off-centre by a cell is a GLOBAL perturbation; three necks at three different x positions would be biased by different amounts and barely at all near the middle. Identical bias at all three means the cause is LOCAL and TRANSLATION-INVARIANT.
2. **Floating-point non-associativity is ruled out.** f32 rounding cannot produce a visible lobe. The user's judgement, and it is right.
3. **Anything that ALTERNATES is ruled out.** Same direction every run, every resolution, over minutes. The sweep parity flips every tick; a drifting bias wanders. Neither produces a fixed lean.

That third point is the sharpest filter and it is what demotes randomness below.

## RANDOMNESS IS DEMOTED -- do not re-run this hypothesis

An earlier version of this ticket named the `dispersion` hash as prime suspect, on the grounds that `disp_roll` takes the LOW 8 bits of `(seed ^ nb_idx*823)` where `seed = x*1299689 ^ y*314159 ^ time_seed*7213`, giving a byte of the form `(233x + A) XOR (55x + B)` -- an XOR of two affine sequences in x, which is not uniform, so the `- 0.5` centring would not actually centre it.

**USER PUSHBACK, and it is correct: "a global rng salt with the hash would essentially randomize the hash and deal with systemic issues like this. randomness is not systematic."** The tick term ALREADY is that salt.

THE ARITHMETIC. `7213 mod 256 = 45`, which is odd, so `45*t mod 256` is a BIJECTION in t -- over any 256 consecutive ticks it hits every residue exactly once. For a fixed x, as `A` sweeps all 256 values, `(233x + A)` sweeps all 256 values, and XOR with a constant is a bijection, so the output is uniform. **The time-average of `dispersion` is therefore exactly zero at every x, over 256 ticks -- about eight seconds.** The piles form over minutes. A systematic per-x dispersion bias cannot survive; the earlier claim was wrong.

The `lock_roll` (physics.rs ~4866) takes 16 bits, so its salt space is 65536 and a full cycle takes ~36 minutes at 30fps rather than 8 seconds. It therefore averages far more slowly and could show sampling DRIFT. But drift wanders -- it does not hold one direction across restarts and across resolutions. Constraint 3 rules it out too.

USER, after seeing this: "now I am not so sure systematic random number bias is our issue." Agreed. Treat randomness as unlikely, but settle it with the experiment below rather than by argument.

## FIRST STEP: the salt experiment (USER'S, and it is better than any check previously proposed here)

Salt every hash from a global RNG seeded ONCE PER RUN, expose the seed in the UI, and run the same scene five times.

- **Lean direction CHANGES across runs** -> randomness is implicated, and you know it without touching the solver.
- **Lean direction IDENTICAL across all runs despite different salts** -> randomness is EXONERATED ENTIRELY and the cause is deterministic structure.

This tests the whole class at once rather than one term, and neither outcome is ambiguous. Expected outcome, on the reasoning above: the second.

NOTE this is a per-RUN salt, not per-tick. A per-tick salt is what already exists. The point of a per-run salt is that it changes the REALIZATION between runs while leaving everything deterministic within a run -- which is exactly the discriminator needed here.

## THEN: the structural suspects, which never alternate and never average

The distinguishing property is FIXED CONVENTION -- something that treats +x and -x differently at every cell, forever.

**1. EDGE OWNERSHIP. Leading suspect.** Every cell owns the edge to its RIGHT. It is the donor-side owner of `(i, i+1)` and merely a participant in `(i-1, i)`. That convention never flips. So anything that treats "my edge" differently from "my neighbour's edge" is a permanent, translation-invariant, same-direction-everywhere bias -- identical at every neck, at every resolution, on every run. That is precisely the observed signature. Check: arbitration order, who writes `cell_avail`/`cell_freecap` versus who reads them, and which edge is served first when a donor is oversubscribed.

**2. GREEDY ARBITRATION.** If an oversubscribed donor serves claimants in index order and clamps as it goes, the lower-index claimant is always served first, permanently. The fix is PROPORTIONAL rescaling, which is order-free by construction. Verify which is implemented before assuming.

**3. TIE-BREAKS.** `<` versus `<=` on `h_a` against `h_b` resolves every exactly-level pair the same way. A settled bed is nothing but exact ties, so this is not a corner case here -- it is the common case.

### The test that covers all three

Extend the antisymmetry check from `flux_edge_candidate` alone up to the WHOLE collect-arbitrate-apply pass. Build a small state, run one tick; mirror the state, run one tick; compare `f(mirror(s))` against `mirror(f(s))`. One tick only -- that is the window where both runs share an input state, so any difference is a definite fact about one pass rather than an accumulated trajectory.

Do it at the pass level, not just the edge function: ownership and arbitration live BETWEEN the per-edge candidate and the apply step, so a clean `flux_edge_candidate` proves nothing about them. (Still worth checking the function alone first, as a one-line sanity check: swap every pair of arguments and negate `v_e_prev`; the result must be the exact negation.)

## SEPARATELY: the hash should be improved anyway, and it is cheap

Not as a fix for this ticket -- as ordinary hygiene. There are FOUR draw sites (`disp_roll` ~4694, `lock_roll` ~4866, `dispersion_noise` ~5265, the flow lock/`alpha_noise` ~5285), each hand-rolled with different magic constants and different bit widths, and all sharing three faults:

- **They take LOW bits.** The low bits of a multiply are the worst-mixed -- carries only propagate upward.
- **They combine by XOR of two multiplies.** Both terms stay affine in x; XOR does not destroy that structure.
- **There is no finalizer.** No avalanche step, so a one-bit input change does not scramble the output.

Fix: ONE shared mixer -- murmur3's `fmix32` or splitmix32 -- applied to a combined key, taking HIGH bits, used by all four sites. About four lines, deterministic, stateless, cheap. Worth doing because three hand-rolled variants is how this class of bug hides, even though the averaging argument above says it is probably not causing THIS defect.

While there: `dispersion` is added to ONE endpoint only (`head_a = ... + dispersion`, no counterpart on `head_b`, in all three driving-head branches). Split it symmetrically (`+d/2` on a, `-d/2` on b) so it perturbs the DIFFERENCE without favouring an endpoint. Correct regardless of hash quality, and it removes the whole class.

## WHY NOT A GLOBAL STATEFUL RNG (asked and answered)

A single stateful stream makes every draw depend on how many draws happened before it. This codebase cannot survive that: the adaptive block scheduler skips blocks and edge sleeping skips edges, so the number of draws per tick varies with WHAT RAN rather than with the physics. `perfect_simulation` would consume a different number of values than the adaptive scheduler and the two would diverge structurally -- and `perfect_simulation_determinism` exists precisely to assert they do not. It would also rule out ever parallelising the solver.

Stateless hashing of `(x, y, tick)` is the correct architecture. A per-run SALT mixed into that hash (the experiment above) keeps every one of those properties while changing the realization -- which is why it is a safe experiment and, if it helps, a safe permanent change.

## One thing to be clear-eyed about

Even a perfect hash is not mirror-EQUIVARIANT: `hash(x) != hash(w-1-x)`. Any randomness breaks exact mirror symmetry of a given realization and no hash fixes that. What a good hash buys is bias that is zero in expectation and decorrelated across x, so it does not accumulate into a persistent lean. You cannot have a symmetric realization WITH noise; you can have noise that does not drift.

(Forcing exact symmetry by keying on `min(x, w-1-x)` would make left and right cells draw identical values -- a correlated artifact of its own. Not recommended.)

## What NOT to do

- Do not re-tune tolerances on `test_water_blob_stays_left_right_symmetric_under_gravity`, ignore it, or retitle it. Deliberate marker; see the handover.
- Do not try another SCAN ORDER. Hilbert and diagonal were tried and only relocate the bias, and there is a principled reason: alternating direction makes the TIME-AVERAGE symmetric, but the solver is nonlinear (yield threshold, donor/acceptor clamps, edge sleeping) and clamping is irreversible, so averaging two asymmetric operators does not produce a symmetric one. Order ROTATION cannot work; only order ELIMINATION can.
- Do not re-run the dispersion-hash-bias hypothesis. The arithmetic above closes it.
- Do not reach for another trajectory-level experiment beyond the salt run. Five have been run and each returned one ambiguous scalar.

## Original framing, still correct and still open

USER POINT 2026-08-05: "pressure is symmetric and it is naturally against assymetry so it makes sense that it reduces assymetry. but it is still a bandaid on underlying assymetry issues that we need to find at some point."

Moving `column_depth` into a standalone unconditional pass (#54 step 1) cut the tick-phase symmetry error ~4x and nearly converged the even/odd runs. That is damping, not a fix, and the 4x is on the SMALL signal only.

## Cross-links

#44 (water drains asymmetrically -- `tau` is 0 for liquid so `dispersion` is identically zero there, meaning that ticket needs a cause that survives with no granular noise at all), #54 (the standalone pass), #45 (pressure projection, which also damps), #70 (if the overfill rewrite happens the yield criterion moves and these terms change role -- re-check rather than assuming this carries over).
