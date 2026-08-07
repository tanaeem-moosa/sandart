# #24 — Rebuild multi-stage as an 8-4-2-1 cascade

**Status:** completed

---

MultiNeck (3 necks) and Staircase (13 steps) are done and pushed in 3b0e7cb2. Serpentine remains — it is the one open-ended design item of the three.

Current: MultiStageHourglass, sandart-sim/src/physics.rs:1041. Three stages whose centre-lines shift -0.12w -> -0.12w -> +0.12w -> 0, blended with smoothstep easing so the slope stays continuous across stage boundaries. Half-width tapers from max_hw = 0.35w down to neck_hw at each stage joint.

User: "we need to make it look nicer but I am not sure how. maybe make it a bunch of smaller and smaller triangles but more and more going up." Read as a graduated cascade — many small triangular chambers, more of them and smaller toward the top, fewer and larger toward the bottom.

Open question worth settling before building: that reading discards the serpentine (side-to-side) character entirely in favour of a fractal/branching funnel. Worth confirming with the user, or prototyping both.

Constraints (all now covered by tests, so they will be caught rather than shipped):
- test_all_sandfall_funnels_conserve_sand_mass — no leaking into MASK_OUTSIDE
- test_serpentine_no_sand_leaking — the older, deeper 500-tick version
- test_flip_inverts_the_structure_not_just_the_sand — the shape must stay a pure function of dy, since Flip now negates it
- Must work across the neck-width slider's full 0.005..0.12 travel and the chamber-curvature slider
