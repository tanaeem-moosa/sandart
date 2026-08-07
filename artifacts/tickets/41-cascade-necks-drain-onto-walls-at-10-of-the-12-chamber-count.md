# #41 — 2.18 — Cascade necks drain onto walls at 10 of the 12 chamber counts

**Status:** completed

---

SHIPPED in 3c661eb6 (pushed to main, live on Pages). All 12 chamber counts now clean.

WAS: only n=8 and n=16 clean. n=5,6,9,10,11,12 had a neck with ZERO open cells below (drains onto solid wall); n=7,13,14,15 had a neck with under a third of its width over open space. User found it at n=7.

CAUSE: each tier laid chambers on its own uniform grid of n_k slots across the full width, independent of the tier above. Alignment happened only by arithmetic luck — true for the power-of-two chain, essentially nowhere else.

FIX: explicit merge tree — a child chamber spans exactly the union of the parents feeding it. Boundaries stay integer multiples of w/n at every tier (exact, integer cross-multiplication, no float drift). Which parents merge is chosen by WIDTH BALANCE, not index.

WHY WIDTH BALANCE MATTERS (the thing that took two rounds): containment in the child's SLOT is not sufficient — the neck must land inside the child's funnel OPENING, which is 0.35*chamber_w about the child's CENTRE. A lopsided child moves its centre away from the parent that fed it. Index-pairing fixed 11 of 12 and left n=9 broken: merging odd twice compounds to [2,2,1,2,2] -> [4,1,4] -> [5,4] in w/9 units, putting a neck 2.0 units off centre against a funnel reaching 1.75 — outside at every row, any curve, any neck width. Width balancing gives [4,3,2] -> [4,5], offsets 1.0/1.5 against 1.75.

Powers of two still reduce to exact uniform halving (verified directly: n=16 gives [0,2..16] -> [0,4,8,12,16] -> [0,8,16] -> [0,16]), so n=8/n=16 are untouched and the bit-identity anchor stayed green unmodified. User had visually validated n=16 before this change, so that validation still holds.

NEW PERMANENT TEST: test_multistage_neck_always_overhangs_open_space_below — every n in 5..=16, every grid size, every tier; asserts no neck has zero open cells below and each has at least a third of its width over open space. Partial overhang is normal (happens at n=8; it is the intended shoulder-and-slide look) and deliberately not asserted away. Non-vacuity verified by reverting the merge rule.

WHY THE OLD TESTS MISSED IT: test_cascade_drains_to_bottom_chamber measured whether sand reached the bottom — it does, by piling against the ridge and spilling over. test_cascade_no_dam_or_neck_merge measured sideways neck overlap. Nothing asserted a neck opens onto anything. Same lesson as the tendril reproduction: the metric measured an adjacent quantity, not the defect.

KNOWN GAP, DELIBERATE: sweep is neck_width {0.06, 0.12} x curve {0.1, 0.6}. Does NOT cover curve > 0.6 at grid 64 with a near-minimum neck — that corner shows single-cell misses one row below a tier boundary even for n=8/n=16, which are bit-identical before and after, so it is pre-existing and orthogonal (at grid 64 a tier is only ~10-16px tall and the taper's vertical rate eats the margin within one row). Test's doc comment says so explicitly.

NOT VERIFIED IN A BROWSER — no working driver on this machine.
