# #56 — 2.33 — Separate the real symmetry fix from pressure damping: does order-independence or restoring force explain the 4x?

**Status:** pending

---

USER POINT 2026-08-05, and it is the right frame: "pressure is symmetric and it is naturally against assymetry so it makes sense that it reduces assymetry. but it is still a bandaid on underlying assymetry issues that we need to find at some point."

## What happened

Moving `column_depth` into a standalone unconditional pass (#54 step 1) cut the tick-phase symmetry error ~4x and nearly converged the even/odd runs:

    even  worst 3.410e-2 -> 8.934e-3    final 1.028e-2 -> 3.295e-3
    odd   worst 3.572e-2 -> 8.775e-3    final 1.254e-2 -> 3.228e-3
    late_persistent_run 75 -> 75 (unchanged; the test still fails)

That is the largest movement #44 has ever seen. It is ALSO exactly what a stronger symmetric restoring force would produce, so it must not be read as a fix until the two candidate causes are separated.

## The two effects are confounded

The change did two separable things at once:

1. **Removed a named order dependence — a genuine cure.** The symmetry test's own failure message calls this out: "`column_depth` is still built from the LIVE `temp_heights` and chains off its own earlier values in the same pass, so it remains order-dependent even after that fix." The standalone pass reads the frozen snapshot, so that is gone.

2. **Made the field non-zero in large static bodies — a bandaid.** Blocks the old scheduler-gated computation never visited silently read ZERO overburden. They now read the real value, which strengthens the depth-driven lateral term everywhere. That term is symmetric and restoring, so it damps ANY lateral asymmetry regardless of its cause. This is also what regressed `test_liquid_flowing_liquid_does_not_stand_in_walls` (voids 5 -> 66): the field changed meaning under a coefficient tuned against the old, partly-zero one.

Nothing measured so far distinguishes (1) from (2).

## DISCRIMINATOR A — nearly free, data already being collected

The `LATERAL_PRESSURE_SCALE` re-sweep now running for #54 reports the symmetry metrics at every swept coefficient value. Read it as an experiment, not just a tuning exercise:

- Symmetry win SURVIVES at the lower coefficient  ->  it came from order-independence. Real. #44 has genuinely moved.
- Symmetry degrades roughly IN PROPORTION to the coefficient  ->  it was damping. #44 is untouched underneath and the number is cosmetic.
- Anything in between  ->  both contribute; report the split rather than picking a story.

Do this reading before anyone updates #44's status.

## DISCRIMINATOR B — the exact control, if A is ambiguous

Make `column_depth` order-independent WITHOUT making it unconditional: keep it scheduler-gated exactly as before, but compute it from the frozen snapshot rather than live `temp_heights`. That isolates effect (1) with effect (2) held fixed, because the field's magnitude and coverage stay as they were.

Whatever symmetry improvement THAT alone produces is the real fix's contribution. The remainder is damping.

This is cheap and it is the clean answer. Prefer it over arguing from A if there is any doubt.

## Why this matters beyond bookkeeping

Two previous #44 fixes (red-black edge colouring, the original `column_depth` freeze) removed order dependence WITHOUT removing the lean — recorded on #45 as "the lean is order-independent and unexplained". If discriminator B shows order-independence again buys little, that pattern holds for a third time and the real cause is somewhere none of the three touched. That would be the most informative possible outcome and should redirect #44 entirely.

Conversely if B buys most of the 4x, then the earlier attempts failed for a reason specific to them (they froze the cross-neighbour READ but left the same-pass CHAINING intact) and order dependence really was the cause all along.

## Do not

Do not mark #44 fixed on the strength of the 4x alone. Do not tune `LATERAL_PRESSURE_SCALE` upward to improve symmetry — that is knowingly buying the bandaid, and it trades against liquid coherence, which is a documented cliff.

Cross-links: #44 (the defect), #54 (where the change came from), #45 (records the two earlier failed order-independence fixes).
