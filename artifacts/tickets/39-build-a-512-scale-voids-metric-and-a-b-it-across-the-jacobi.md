# #39 — 2.16 — Build a 512-scale voids metric and A/B it across the Jacobi fix

**Status:** pending

---

Raised 2026-07-31. The user reports that water in the TOP HALF of the container behaves better than they remember — no longer leaving large holes as it falls out — observed at 512, and asks whether we changed something.

## The puzzle
`7a3ef9f` (Gauss-Seidel -> Jacobi driving on the gravity lateral edge) is the ONLY physics change affecting 512 in that window. `cd53453` (resolution invariance) is an exact no-op at 512 by construction and verified; `d6e965b` (grid switch), `f652ec3` (quantile/colour) and `8c04f648` (detector) contain no physics.

But `7a3ef9f` moved the walls metric the WRONG way: enclosed-void total 12106 -> 17570. That metric counts holes. So the number says holes got worse while the user says they got better.

## Why the metric is not automatically right
Every liquid metric in this project runs at **64x64**. One of them has already misled us badly: `test_multineck_hourglass_water_tendril_on_impact` measures a solid splash pool (`filled/bounding = 1.000` at every resolution) while the user was reporting one-cell hairlines — and SIX fix formulations were evaluated against it before that was noticed.

A 64-cell hourglass drain is not obviously the same phenomenon as water falling through the top half of a 512 container. We currently have NO trustworthy production-scale measurement of what the user is describing.

Note also that the walls test FAILS outright at 512 (66.7M against a 34,000 bound) for reasons diagnosed in #37 — so it cannot be used at production scale as-is anyway.

## The task
1. Build a metric that measures what the user actually describes: enclosed voids / holes in the UPPER REGION during active drain, at 512, in a container they actually use. Follow the pattern of the tendril detector (`find_liquid_components`, `TENDRIL_THRESHOLDS`) — connected-component analysis with stated thresholds and a sensitivity check — rather than reusing the 64-scale walls counter.
2. A/B it across `7a3ef9f`: measure with the current frozen-snapshot driving, then with the driving reverted to the live `temp_heights` buffer, then revert the experiment. Report both.
3. Say plainly whether the Jacobi fix improved, worsened, or did not affect the thing the user sees.

## Why this matters beyond answering one question
If the Jacobi fix improved the VISIBLE behaviour while worsening the 64-scale metric, then that metric is actively misleading about production, and every decision we have justified with it needs re-examining — including the trade-off recorded in `7a3ef9f`'s own commit message, which I framed to the user as a real cost. Getting this wrong in the other direction is equally bad: if the metric is right and the improvement is imagined, we should know before building on it.

## Must not regress
- Do NOT weaken assertions; do not touch the intentionally-red test.
- Revert the A/B experiment; ship only the metric.
- Sand bit-identical.
- Mass: measured band 1e-9 to 1e-8.
