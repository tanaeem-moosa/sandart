# #46 — 2.23 — Graininess: randomized-weighted property/colour transfer (NOT surface shape)

**Status:** completed

---

GOAL (user's words, and they corrected me twice to get here): graininess means randomness in COLOUR AND PROPERTY MIXING. Not surface shape, not flow scatter, not the renderer. "not just color. other properties too. that's why I wanted randomized weight from each side."

THE SPEC, in the user's own example: three edges contributing 1, 0.5, 1 against a budget of 1. Proportional gives .4/.2/.4. Randomized-weighted should give three DIFFERENT numbers that still sum to 1, as a function of a few random numbers, the grain size, and those three ratios. Explicitly NOT "pick one donor and take its claim, which may be less than the budget" — that is a different mechanism and is not what was asked for.

WHERE IT LANDS, and why this is one change rather than two: the arbitration limiter, not `advect_properties`. The flux magnitudes ARE the blend weights that `advect_properties` uses for both `colors` (3 channels) and `props` (4). Randomising how a contested budget is split therefore randomises colour and property mixing together, automatically. My earlier design in this task — randomized SELECTION inside `advect_properties` — was the wrong place and the wrong mechanism; superseded.

IMPLEMENTED (working copy, not pushed): per-edge multiplicative jitter `r`, share becomes |raw_e|·r_e / Σ|raw_e'|r_e'. New: `budget_term`, `grain_jitter_strength`, `edge_share_jitter`, `accumulate_edge_totals`, `GRAIN_JITTER_SCALE = 1.25`, `GRAIN_JITTER_MAX = 0.95`, plus jittered totals alongside the raw ones.

TWO SOUNDNESS POINTS worth not re-deriving:
- Oversubscription is tested against the RAW total, the split taken against the JITTERED one. Testing with the jittered total would be unsound: jitters below 1 shrink it, so `jit_total <= budget` does not imply the raw claims fit.
- The Zalesak single-pass proof carries over verbatim. It only ever required each side's per-edge factors to respect that side's own budget SUM — it never required every edge sharing a cell to get the SAME factor. That slack is the whole mechanism.

WHY THE PARKED ATTEMPT (429f5111) WAS SLOW, since the user rightly pushed back on "randomising is expensive": it discretized to two extreme points, which needed pair coordination, an `h_pending` buffer, a degree<=2 restriction and a fallback path. This design has none of that — one hash and a multiply per edge, no extra pass. Perf being bad again would be an implementation fault, not an inherent cost.

STATUS: compiles; suite 89 pass / 1 intentional fail / 14 ignored, and the intentional failure's metrics are numerically IDENTICAL to before the change, confirming the liquid gate is bit-exact as designed. Sonnet subagent now measuring the deciding metric — colour/property variance A/B via GRAIN_JITTER_SCALE 0.0 (bit-identical to main) vs 1.25 — plus perf at 512 and the drain-order sweep.

GATE (user's standing instruction): if measured variance does NOT increase, park this and move to 2.22 pressure. Do not iterate on a further graininess mechanism.
