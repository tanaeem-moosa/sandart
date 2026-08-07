# #52 — 2.29 — Material forms straight VERTICAL lines: no lateral mixing in the draining funnel

**Status:** pending

---

USER REPORT 2026-08-04, MultiNeck photo: "this straight lines are kinda concerning. why do they not move sideways. maybe a bug in sand water boundary." Clarified: "I don't mean the quantitie overlay line. I mean the material making a straight vertical line."

Photo: `/home/deck/.claude/uploads/6dbad8f7-de15-4c1a-aae8-0d4d41f500d8/22c8fe3f-20260804_1942183593398147278752967.jpg`
Crops: `artifacts/design/lines/center_stripes.png`, `artifacts/design/lines/left_pile.png`.

## SETTLED 2026-08-05: this is PHYSICS, not the scheduler

User, after testing the perfect-simulation toggle (18878a8): "perfect simulation creates the vertical lines this not a simulation artifact."

Read as: the vertical lines are present under perfect simulation too, therefore the block-LOD scheduler is not the cause. (If instead the toggle INTRODUCED lines that were absent before, that is a bug in the toggle itself and this section is wrong — but that reading contradicts "this not a simulation artifact", so it is not the one taken.)

This is the clean counterpart to the same experiment on #47, and together they partition the two defects:

- **Slabs (#47): scheduling.** Gone under perfect simulation.
- **Vertical lines (this): physics.** Present under perfect simulation.

Consequence: stop looking at activation, budget, staleness or block boundaries for this defect. The cause is in the driving terms. That is exactly what #54 is working on, and effect 2 there ("material next to empty space spreads into it") is the most likely fix.

STRONGEST SUSPECT, carried over to #54's step 2: for sand the update is `yielded = sign(dH) * max(|dH| - tau, 0)`. If the yield stress `tau` is comparable to the lateral driving head, sand cannot move sideways AT ANY DEPTH — the driving term is subtracted away before it can act, which would produce exactly a body that advects straight down in independent columns and never mixes across them. `tau` is 0 for liquid, so this predicts a sand/water split in spreading. Measure it.

## THE REPRO — user-supplied 2026-08-04

"I can recreate it in different ways. for example water of the left and wet sand on the right with linear gradient causes on top. this one was on the bottom."

**Place WATER on one side and WET SAND on the other, linear-gradient colour scheme.** A straight vertical line appears; position varies (top there, bottom in the MultiNeck photo). Reproduces across containers, colour schemes and positions, with no funnel, no convergence and no orifice.

## SWAPPING THE SIDES DOES NOT MIRROR THE RESULT

User: "water left, wet sand right. it happens right here the right most hourglass starts. but wet sand left water right, it happens almost all the way to the left. so this behavior is also assymetric."

### 1. In both runs the line ends up on the SAND side, so the interface MIGRATES

Right of centre when water starts left, left of centre when water starts right — either way it has moved into what started as sand. Not a pinned boundary sitting where it was painted: water is displacing sand and the line is the front. This RULES OUT rejection of unlike-material transfer, which could not migrate at all. What is left is transfer across a water|sand edge that happens but is BIASED.

### 2. The migration DISTANCE is asymmetric — a handle on #44's left bias

Model front displacement as D (displacement) plus L (the #44 left bias):

- water left / sand right: front moves RIGHT, bias opposes -> (D - L)
- sand left / water right: front moves LEFT, bias assists -> (D + L)

Two runs, two unknowns. D = ((D+L) + (D-L))/2, L = ((D+L) - (D-L))/2 — a SIGNED, SCALAR measurement of the left bias in cells.

HYPOTHESIS THAT FITS, NOT A RESULT. Fitted to an eyeball description; "almost all the way to the left" is not a measurement. It predicts that mirroring the ENTIRE setup flips which run travels further, and that L agrees with #44's other expressions. If L measures near zero, this section is wrong. Always run as a mirrored pair.

## Hypotheses, in current order

1. **Yield stress cancelling the lateral driving head for sand** — see the SETTLED section. Test first.
2. **Asymmetric unlike-material lateral transfer.** The interface migrates, so transfer happens but is biased. Check whether `head_a`/`head_b` stay consistent across an edge whose two sides differ in `cell_liquidity` — `column_depth` "stays gated on `cell_liquidity > 0.0`" (physics.rs ~3198), so the two head terms are built from different quantities.
3. **The granular factor on lateral pressure** (physics.rs ~331-337). If it scales the sand side and not the water side, the edge sees a discontinuous coefficient.
4. **Wetness/liquidity not diffusing laterally.** A sharp vertical wetness edge would show as a line via shading rather than mass. Dump wetness separately from mass.

## Under a colour scheme the paint is a passive TRACER

Concentric rings and linear gradients are smooth and curved-or-diagonal by construction. A straight VERTICAL feature is in neither, so it is drawn by the flow or the front, not the paint.

## THREE DISTINCT FEATURES IN THE PHOTO — do not conflate

1. **Thin HORIZONTAL threads** — the quantile overlay (#10). They cross empty black space, and shader.wgsl:763-778 draws them from `row_f = uv.y * grid_size` with NO x term. Not material.
2. **FINE COMB of near-constant pitch with cyan/magenta fringing** — the NEGATIVE TENDRILS of #44. I called these LCD moiré; that was WRONG. User: "they seems to move", which moiré cannot. Constant pitch at the grid scale means a wavelength set by the discretisation — a grid-scale odd-even mode, hence the edge-vs-collocated constraint on #45. Tracked under #44/#45, not here.
3. **COARSER IRREGULAR VERTICAL STREAKS** in the converging zone — THIS ticket, together with the side-by-side front.

## The decisive test — no camera, runnable now

ALWAYS AS A MIRRORED PAIR:
- Half water, half wet sand, linear gradient. Headless. Dump MASS, MATERIAL and WETNESS separately each tick.
- Per row, locate the front's x; report mean, spread across rows (is it really a straight line?), migration over time.
- Extract D and L; state the sign convention.
- At 64 AND 512, so a resolution dependence would show.

## Do not

Do not nudge `LATERAL_PRESSURE_SCALE` blindly or add ad-hoc lateral diffusion of the colour field. Diffusing the tracer hides the symptom, leaves the transport wrong, and corrupts the instrument that made this visible.

Cross-links: #54 (most likely fix), #44 (left drift, tendrils), #47 (the same experiment, opposite result), #42 (closed), #33 (open).
