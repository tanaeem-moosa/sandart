# #44 — 2.21 — Water drains asymmetrically; left drift. CAUTION: pressure MASKS this, do not mistake damping for a fix

**Status:** pending

---

USER REPORT after 25bc7368 (Jacobi). Water drains asymmetrically from the top, with small surface waves mostly travelling LEFT. User rates it much less serious than the tendrils it replaced and prefers the current state.

## 2026-08-04: #52 GIVES THIS TICKET ITS FIRST QUANTITATIVE INSTRUMENT — READ THAT FIRST

#52 (vertical line at a water/sand boundary) turned out to carry a mirrored-pair experiment that measures THIS defect as a signed scalar. User: "water left, wet sand right. it happens right here the right most hourglass starts. but wet sand left water right, it happens almost all the way to the left. so this behavior is also assymetric."

Swapping the two materials does NOT mirror the result — the leftward run travels much further than the rightward one. Modelling the front displacement as D (water displacing sand) plus L (this ticket's left bias) gives (D - L) for water-left/sand-right and (D + L) for sand-left/water-right, so two runs solve for BOTH. That is a measurement of L in cells, replacing the qualitative "waves drift left" this ticket has run on since it was opened. Full method, sign conventions and falsification criteria are in #52 — do not duplicate them here.

Every other expression below is qualitative. Get L measured before spending solver time on any of them.

## UPDATE 2026-08-01: additional symptom, same signature

With the bottom tendrils now fixed (#34 closed), the user reports subtle NEGATIVE tendrils remaining at the TOP, drifting toward the LEFT. Same left-drift direction as the wave report above, so treat it as one defect with two visible expressions rather than two bugs. User expects pressure projection (#45) to improve it.

DEFERRED by the user 2026-08-01: "too complex for 10 percent" of the weekly quota. Not to be started this week. Sequenced after #45 (pressure).

## UPDATE 2026-08-04: THIRD expression — MultiNeck, all three streams drift LEFT

USER REPORT with photo: "I found another assymetry. mutlineck. water and wet sand as concentric ring. all three are just falling to the left."

Photo: `/home/deck/.claude/uploads/6dbad8f7-de15-4c1a-aae8-0d4d41f500d8/22c8fe3f-20260804_1942183593398147278752967.jpg`
On-screen HUD: MultiNeck, 3 necks, concentric-ring colour scheme, 24.1 ms/frame, 408 blocks, build stamp `530S79840 2026-08-02 05:06 UTC` (i.e. main at 530579b4, the rename commit).

MY READING OF THE IMAGE, flagged as a reading and not a measurement: each of the three lower chambers holds a pile whose peak sits at or right of its neck, with a long shallow tail running down-LEFT and a steep face on the right. Same handedness in all three. The upper chamber's surface is likewise tilted rather than mirror-symmetric.

### Why this report is more useful than the previous two

1. It is NOT water-only. Wet sand shows it too. #44 was framed as a liquid-solver asymmetry, and the strong prior below (tick-phase / `column_depth` cross-neighbour read) lives in the liquid path. If sand drifts left by the same amount and in the same direction, either the mechanism is shared with the sand path or it is upstream of both (mask, or the block scheduler's traversal order). CHECK THIS FIRST — it partitions the search space in one run.

2. Three necks at three different x positions all drift the SAME way. If the bias came from per-neck geometry in a mirror-symmetric mask, the outer pair should drift in OPPOSITE directions (left neck left, right neck right). Uniform left across all three is what a GLOBAL left-handed bias looks like. It does not rule the mask out — a rasterisation bias that floors every neck centre would also shift all three the same way — but it does mean the mask diff must be against the GLOBAL mirror about cell w/2, per neck, not neck-against-its-own-neighbour.

3. It gives the OWED mask check below a concrete container to run against, and MultiNeck is now the cheapest repro on file: three independent samples of the same bias in one frame.

### RULED OUT 2026-08-04: the straight HORIZONTAL lines in that photo are the quantile HUD, not material

User asked about thin straight lines across the material: "this straight lines are kinda concerning. why do they not move sideways. maybe a bug in sand water boundary." I first answered with the horizontal ones, which ARE the mass-distribution quantile overlay from #10 and carry no information about this defect:

- CROP EVIDENCE: they continue past the material edge and across EMPTY BLACK SPACE (`artifacts/design/lines/right_black.png`; the top line in `artifacts/design/lines/upper_gap.png` crosses the void between the humps). Material cannot exist as a 1px horizontal thread in a vacuum.
- CODE: shader.wgsl:763-778 draws them from `row_f = uv.y * grid_size`. NO x term, so they are horizontal by construction and cannot move sideways. Colour `vec3(0.561, 0.722, 0.631)` matches. Drawn after the `in_casing` early-return, hence confined to the shape interior — which is why lines have different lengths.

Do not read shear or tilt off those lines.

THE USER MEANT THE VERTICAL ONES, which are a different defect entirely and are now tracked in #52.

## STRONG PRIOR: this is the known residual tick-phase asymmetry, and a fix already exists

`test_water_blob_stays_left_right_symmetric_under_gravity` still fails on main, and its even/odd runs still DIFFER (worst 1.643e-2 vs 5.041e-2, late_persistent_run 46 vs 75), so the sim is still not tick-phase invariant. Two of eight mechanisms still move it.

An overnight experiment freezing `column_depth`'s cross-neighbour read achieved FULL bit-for-bit invariance across all eight mechanisms — confirming the mirrored-edge-sequencing argument (the symmetry test mirrors with j = w - x, so its axis is the CENTRE of cell w/2; a lateral edge between (i, i+1) mirrors to left-cell 63-i, and i and 63-i always have OPPOSITE parity, so no parity-based colouring can ever pair an edge with its mirror — but a frozen read makes them exactly antisymmetric). It was reverted only because it regressed liquid stream max_width 7->9 and flowing-liquid voids 0->23. Documented in place: search "EXPERIMENT, TRIED AND REVERTED" in physics.rs.

KEY POINT: those regression figures were measured against the PRE-Jacobi baseline. Plain Jacobi has since ALSO moved stream max_width to 9. If the freeze gives 9 too, it costs nothing extra on that metric and only the voids are new. Being re-measured now on top of current main.

So resolving `test_liquid_stream_stays_coherent` is not hygiene — it is the GATE on fixing this asymmetry.

CAVEAT ADDED 2026-08-04: this prior is a LIQUID-path explanation. The wet-sand observation above may not be covered by it. Do not assume one fix closes all expressions until the sand-vs-water comparison is actually run. Note the #52 instrument uses a water|sand INTERFACE, so it exercises both paths at once — if the freeze fixes L there, that is strong evidence; if it does not, the prior is wrong.

## OWED: MultiNeck mask symmetry check

Still not done, and still the cheapest step. Rasterise the MultiNeck container mask and diff it against its own mirror about cell w/2, at each neck and for the chamber walls. It would rule out the geometry itself being asymmetric before any more solver work is spent. Run at the photo's resolution and at 512.

## SEPARATE, HARDER: the 45-degree triangle

User also reports a 45-degree triangular feature forming where water is NOT falling, and suspects it needs proper pressure simulation. Agreed, and the ANGLE is the diagnostic: an explicit scheme propagates influence one cell per tick in each direction, so its domain of dependence is a 45-degree cone. Real pressure is elliptic and propagates across the whole fluid instantly. A 45-degree feature is that numerical characteristic cone made visible, not a physical shape.

This will NOT yield to tuning, ordering, or arbitration — it is the same propagation gap that limits drain order. It needs a pressure projection. Track under #45. Do not attempt local fixes for it.

## PRIORITY

User explicitly ranks this well below the tendrils it replaced and is happy with the trade. Ordering agreed with user 2026-08-01: slabs (#47) now -> pressure (#45) next week -> then repose -> then this. Unchanged by the 2026-08-04 reports, which the user filed rather than escalated ("file as part of symmetry investigation"). The #52 instrument is cheap and does not need pressure, so it can run ahead of that ordering without disturbing it.
