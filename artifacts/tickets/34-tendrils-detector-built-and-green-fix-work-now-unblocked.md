# #34 — 2.11 — Tendrils: detector BUILT and green; fix work now unblocked

**Status:** completed

---

FIXED by the frozen-state Jacobi conversion. Bottom tendrils are gone — user confirmed visually on the Pages deployment 2026-08-01. The detector (`test_single_neck_hourglass_water_tendril_on_impact`) no longer fires at any threshold sensitivity; it fired for 3 ticks with max_length 7 before. MultiNeckHourglass peak_column_depth dropped 133.9 -> 53.0 and the driving-head ratio 669x -> 265x. Specificity guard stays green.

RESIDUAL, not part of this task: the user still sees subtle NEGATIVE tendrils at the TOP, drifting toward the left. That is the same left-drift signature as #44 and is recorded there. Expected to be improved by pressure projection (#45).
