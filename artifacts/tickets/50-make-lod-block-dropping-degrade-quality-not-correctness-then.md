# #50 — 2.27 — Make LOD block-dropping degrade quality, not correctness; then reduce how often it drops

**Status:** pending

---

Strategy agreed with user 2026-08-02, after slabs (#47) survived the upstream-activation fix and proved INTERMITTENT — strong evidence that budget saturation, not activation topology, is what remains.

## USER'S DECISION (2026-08-02), start here

"that is an argument for making all potentially active blocks must simulate, at least in gravity mode."

Promote genuinely-active blocks to the Fast / must-simulate class in gravity mode, so they bypass `budget_n` rather than competing for it. This is "overspend the budget" expressed as a tier change.

NOTE this reverses an earlier proposal (upstream -> Fast) that the user rejected on 2026-08-01, on the sound grounds that active blocks already sit at Medium/Slow and already compete for budget, so competition was accepted behaviour rather than a new failure mode. What changed is EVIDENCE, not reasoning: the intermittency shows the budget genuinely binds during a flip, so competing is exactly where the corruption comes from. Not a contradiction — different facts.

**OPEN QUESTION TO SETTLE FIRST: what counts as "potentially active"?** Defined too broadly this degenerates into simulating everything, i.e. paying the full cost of removing LOD without having decided to. Define it tightly, and measure the resulting block count during a flip before committing.

## Why this is the right shape of fix

The flaw is NOT that LOD drops needed work in general. A settled pile is genuinely inert and LOD is free value there — mean Medium-tier blocks measured 71.7 against a budget of 256, so most of the time the budget does not bind. It binds during a FLIP, when the whole body is released at once. That matches the report exactly: slabs on some flips, not all.

The flaw is that when the active set legitimately exceeds the budget, the scheduler silently drops blocks and CORRUPTS the result instead of admitting it is over budget. A flip that hitches for two or three frames is invisible; sand hanging in mid-air is not. The current design trades a permanent visual artifact for a frame-rate guarantee, which is backwards for a piece whose entire point is how it looks.

Far cheaper than removing LOD outright, which at 512 would be roughly 4x the work every tick, permanently, to fix something that only bites during transients.

### Sizing measurement this depends on
How far over budget does a flip actually go, and for how many ticks? 2x for 3 ticks makes overspending obviously right; 10x for 100 ticks does not. Same instrumentation pass as the budget-saturation measurement owed in #47 — one job answers both.

### Fallback if overspending proves too expensive
Make skipping a QUALITY knob rather than a physics knob — carry previous flux forward for a skipped block, or make support a checked invariant so unsupported material cannot persist regardless of scheduling. Correctness must not depend on the budget.

## Second goal: reduce how often it drops (performance)
Raises the threshold, does not remove the failure mode — it returns on slower hardware, busier scenes, and especially at 1024 (#38). Worth doing, not a substitute.

## Coupling — changes sequencing expectations
Pressure (#45) and acceleration (#49) BOTH add per-tick cost, making saturation MORE likely. Slabs may get worse before they get better; do not read that as the pressure work failing.

The upstream-activation fix already cost +9.3% ms/tick at 512 (9.09 -> 9.94) for a partial result; whether to keep it is open pending #47's measurement.

## Perf baseline is not trustworthy yet
Figures do not reconcile: 2.71, 5.33, 9.09/9.94 ms/tick from different agents under different bench settings, against 24.3 ms/frame observed in the actual app. Establish ONE credible benchmark matching what the app does before optimising against any of them.

## Sequence
#45 pressure -> #47 slabs + budget saturation -> #49 acceleration -> this -> #44 symmetry.
